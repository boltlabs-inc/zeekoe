# To install requests library, run the following:
# pip install requests
#
# To setup the merchant's sandbox config and start the merchant server, run the following:
# $: python3 test-zeekoe.py merch-setup --url "http://localhost:20000" -v
# 
# Then setup the customer's sandbox config and start the chain watcher, run the following:
# $: python3 test-zeekoe.py cust-setup --url "http://localhost:20000" -v
# 
# To run all the scenario tests, run:
# $: python3 test-zeekoe.py test-all
# 
# Then test the life cycle of a few channels (ideally in parallel): establish a channel, make a payment and run cust close
# $: python3 test-zeekoe.py scenario --channel 1 -v --command-list establish pay pay pay_all close
#
# To test a dispute scenario, where the customer closes on a revoked state, use 'store' and 
# 'restore' to restore a revoked state, e.g.
# $: python3 test-zeekoe.py scenario --channel 1 -v --command-list establish pay store pay restore close
# 
# List the channels
# $: python3 test-zeekoe.py list
#

import argparse
import glob
import json
from pprint import pprint
import os
import random
import requests
import shutil
import subprocess
import sys
import time

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'
BBlack='\033[1;30m'

TESTNET = "testnet"
SANDBOX = "sandbox"

MERCH_SETUP = "merch-setup"
CUST_SETUP = "cust-setup"
SCENARIO = "scenario"
TEST_ALL = "test-all"
LIST = "list"

ESTABLISH = "establish"
PAY = "pay"
PAY_ALL = "pay_all"
MUTUAL_CLOSE = "mutual_close"
CLOSE = "close"
EXPIRE = "expire"
STORE = "store"
RESTORE = "restore"

ZKCHANNEL_CUST_BIN = ["../target/debug/zkchannel", "customer"]
ZKCHANNEL_MERCH_BIN = ["../target/debug/zkchannel", "merchant"]

# The minimum blockchain level to be able to run tests. Operations need to reference a block up to 
# 60 blocks from the head. Setting this minimum level avoids running into errors caused by the 
# blockchain not having enough blocks. 
MIN_BLOCKCHAIN_LEVEL = 60

def info(msg):
    print("%s[+] %s%s" % (GREEN, msg, NC))

def log(msg, debug=True):
    if debug: print("%s%s%s" % (BBlack, msg, NC))

def err(msg):
    print("%sERROR?%s %s%s%s" % (BBlack, NC, RED, msg, NC))

def fatal_error(msg):
    print("%sERROR:%s %s%s%s" % (BBlack, NC, RED, msg, NC))
    sys.exit(-1)

def create_merchant_config(merch_db, merch_config, merch_account_keys, self_delay, confirmation_depth, url_path, verbose=False):
    config_contents = """
database = {{ sqlite = "{merchant_db}" }}
{tezos_account}
tezos_uri = "{url}"
self_delay = {self_delay}
confirmation_depth = {confirmation_depth}

[[service]]
address = "::1"
private_key = "localhost.key"
certificate = "localhost.crt"

[[service]]
address = "127.0.0.1"
private_key = "localhost.key"
certificate = "localhost.crt"    
    """.format(merchant_db=merch_db, tezos_account=merch_account_keys, self_delay=self_delay, confirmation_depth=confirmation_depth, url=url_path)
    f = open(merch_config, "w")
    f.write(config_contents)
    f.close()
    info("-> Created merchant config: %s" % merch_config)
    if verbose:
        print("============")
        print(config_contents)
        print("============")
    return
    
def create_customer_config(cust_db, cust_config, cust_account_keys, self_delay, confirmation_depth, url_path, verbose=False):
    config_contents = """
database = {{ sqlite = "{customer_db}" }}
trust_certificate = "localhost.crt"
{tezos_account}
tezos_uri = "{url}"
self_delay = {self_delay}
confirmation_depth = {confirmation_depth}
    """.format(customer_db=cust_db, tezos_account=cust_account_keys, self_delay=self_delay, confirmation_depth=confirmation_depth, url=url_path)
    f = open(cust_config, "w")
    f.write(config_contents)
    f.close()
    info("-> Created customer config: %s" % cust_config)
    if verbose:
        print("============")
        print(config_contents)
        print("============")
    return

def run_command(cmd, verbose):
    process = subprocess.Popen(cmd, start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    output_text=""
    while True:
        try:
            output = process.stdout.readline()
            if output:
                output_text = output.strip().decode('utf-8')
                log("-> %s" % output_text, verbose)
            error = process.stderr.readline()
            if error:
                err("-> %s" % error.strip().decode('utf-8'))
            if process.poll() is not None:
                break
        except KeyboardInterrupt:
            process.terminate()
    rc = process.poll()
    return output_text, rc

def zkchannel_merchant(*args, config, verbose, **kwargs):
    cmd=[]
    cmd.extend((*ZKCHANNEL_MERCH_BIN, "--config", config))
    for a in args:
        cmd.append(a)
    for k, v in kwargs.items():
        cmd.append(f"--{k}")
        cmd.append(f"{v}")
    return run_command(cmd, verbose)

def zkchannel_customer(*args, config, verbose, **kwargs):
    cmd=[]
    cmd.extend((*ZKCHANNEL_CUST_BIN, "--config", config))
    for a in args:
        cmd.append(a)
    for k, v in kwargs.items():
        cmd.append(f"--{k}")
        cmd.append(f"{v}")
    return run_command(cmd, verbose)

def get_blockchain_level(url):
    full_url = url + "/chains/main/blocks/head/metadata"
    r = requests.get(url = full_url)
    data = r.json()
    level = data['level']['level']
    return level

def check_blockchain_maturity(url):
    level = get_blockchain_level(url)
    while level < MIN_BLOCKCHAIN_LEVEL:
        blocks_short = MIN_BLOCKCHAIN_LEVEL - level
        wait_secs = blocks_short*2
        print(f"Blockchain level is {level} but needs to be at least {MIN_BLOCKCHAIN_LEVEL}. Reattempting in {wait_secs}s")
        time.sleep(wait_secs)
        level = get_blockchain_level(url)

class TestScenario():
    def __init__(
            self, 
            cust_config, cust_db, 
            merch_config,
            config_path,  
            channel_name, customer_deposit, 
            verbose
        ):
        self.cust_config = cust_config
        self.merch_config = merch_config
        self.config_path = config_path
        self.cust_db = cust_db
        self.temp_path = os.path.join(config_path, "temp")
        self.channel_path = os.path.join(self.temp_path, f"{channel_name}")
        self.channel_name = channel_name
        self.customer_deposit = float(customer_deposit)
        self.balance_remaining = float(customer_deposit)
        self.verbose = verbose

         # Create temporary directory to store revoked customer state when testing dispute scenarios
        if not os.path.isdir(self.temp_path):
            os.mkdir(self.temp_path)

    def transfer_db_files(self, src, dst, db_name):
        """transfer all db files '-shm' and '-wal' """
        db_path = os.path.join(src, db_name)
        for file in glob.glob(db_path + '*'):
            db_name = os.path.basename(file)
            # set path for destination db file
            new_file = os.path.join(dst, db_name)
            shutil.copyfile(file, new_file)

    def get_channel_id(self):
        # Load the customer db in json format
        db_data, _ = zkchannel_customer(
            "list", 
            "--json",
            config=self.cust_config, 
            verbose=self.verbose
            )
        d = json.loads(db_data)
        for i in d:
            if i['label'] == self.channel_name:
                channel_id = i['channel_id']
                break
        return channel_id

    def run_command_list(self, command_list):
        for command in command_list:
            if command == ESTABLISH:
                info(f"Creating a new zkchannel: {self.channel_name}")
                initial_deposit = "{amount} XTZ".format(amount=str(self.customer_deposit))
                print('initial_deposit', initial_deposit)
                zkchannel_customer(
                    "establish",
                    "zkchannel://localhost",
                    config = self.cust_config,
                    verbose = self.verbose, 
                    label = self.channel_name,
                    deposit = initial_deposit
                    )
                    
            elif command == PAY:
                max_pay_amount = self.balance_remaining / 2 # save money for future payments
                pay_amount = round(random.uniform(0, max_pay_amount), 4)
                self.balance_remaining -= pay_amount
                payment = "{amount} XTZ".format(amount=str(pay_amount))
                info(f"Making a {payment} payment on zkchannel: {self.channel_name}")
                zkchannel_customer(
                    "pay", 
                    self.channel_name,
                    payment,
                    config=self.cust_config,
                    verbose=self.verbose
                    )

            elif command == PAY_ALL:
                pay_amount = self.balance_remaining
                self.balance_remaining = 0
                payment = "{amount} XTZ".format(amount=str(pay_amount))
                info(f"Paying the remaining balance ({payment}) on zkchannel: {self.channel_name}")
                zkchannel_customer(
                    "pay", 
                    self.channel_name,
                    payment,
                    config=self.cust_config,
                    verbose=self.verbose
                    )

            elif command == CLOSE:
                info("Initiate closing on the zkchannel: %s" % self.channel_name)
                zkchannel_customer(
                    "close", 
                    "--force",
                    self.channel_name,
                    config=self.cust_config, 
                    verbose=self.verbose
                    )

            elif command == STORE:
                log("Storing customer state with remaining balance of %s" % self.balance_remaining)
                # Create temporary directory to store revoked customer state when testing dispute scenarios
                if not os.path.isdir(self.channel_path):
                    os.mkdir(self.channel_path)
                self.transfer_db_files(src = self.config_path, dst = self.channel_path, db_name = self.cust_db)

            elif command == RESTORE:
                log("Restoring customer state")
                self.transfer_db_files(src = self.channel_path, dst = self.config_path, db_name = self.cust_db)

            elif command == MUTUAL_CLOSE:
                    info("Initiate mutual close on the zkchannel: %s" % self.channel_name)
                    zkchannel_customer(
                        "close",
                        self.channel_name,
                        config=self.cust_config,
                        verbose=self.verbose
                        )

            elif command == EXPIRE:
                channel_id = self.get_channel_id()
                info("Initiate expiry on the channel id: %s" % channel_id)
                zkchannel_merchant(
                    "close", 
                    config=self.merch_config, 
                    verbose=self.verbose, 
                    channel=channel_id
                    )
            else:
                fatal_error(f"{command} not a recognized command.")


COMMANDS = [LIST, MERCH_SETUP, CUST_SETUP, SCENARIO, TEST_ALL]
def main():
    parser = argparse.ArgumentParser(formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("command", help="", nargs="?", default="list")
    parser.add_argument("--config-path", help="path to create configs", default=".")
    parser.add_argument("--network", help="select the type of network", default=SANDBOX)
    parser.add_argument("--self-delay", "-t", type=int, help="self-delay for closing transactions", default="120")
    parser.add_argument("--confirmation-depth", "-d", type=int, help="required confirmations for all transactions", default="1")
    parser.add_argument("--url", "-u", help="url for tezos network", default="http://localhost:20000")
    parser.add_argument("--amount", "-a", help="starting balance for each channel", default="10")
    parser.add_argument("--verbose", "-v", help="increase output verbosity", action="store_true")
    parser.add_argument("--channel", type=int, help="desired starting channel counter", default="1")
    parser.add_argument('--command-list','-c', nargs='+', help='''
        Commands to be tested. The list of valid commands and their descriptions are:
        establish - creates a new zkChannel
        pay - pays the merchant a random amount (at most spending half the remaining balance)
        pay_all - pays the merchant the full remaining balance in the channel
        close - performs a customer-initiated unilateral close on the channel
        store - saves the customer db files in a channel-specific directory under temp_path.
        restore - restores the customer db files saved during 'store'. This overwrites the existing customer db. 
        ''')

    args = parser.parse_args()

    if args.command not in COMMANDS:
        fatal_error("'%s' not a recognized command. Here are the options: %s" % (args.command, COMMANDS))
    
    verbose = args.verbose
    config_path = args.config_path
    url = args.url.lower()
    network = args.network.lower()

    self_delay = args.self_delay
    confirmation_depth = args.confirmation_depth
    customer_deposit = args.amount
    channel_count = args.channel
    command_list = args.command_list

    if int(channel_count) <= 0:
        fatal_error("Expected '--channel' to be > 0")

    if network not in [SANDBOX, TESTNET]:
        fatal_error("Specified invalid 'network' argument. Values: '%s' or '%s'" % (SANDBOX, TESTNET))

    cust_config = os.path.join(config_path, f"Customer-{network}.toml")
    cust_db = f"customer-{network}.db"
    merch_config = os.path.join(config_path, f"Merchant-{network}.toml")
    merch_db = f"merchant-{network}.db"
    channel_name = f"my-zkchannel-{str(channel_count)}"

    if network == SANDBOX:
        cust_keys = "tezos_account = { alias = \"alice\" }"
        merch_keys = "tezos_account = { alias = \"bob\" }"        
    elif network == TESTNET:
        cust_keys = "tezos_account = '../../tezos-contract/pytezos-tests/sample_files/tz1iKxZpa5x1grZyN2Uw9gERXJJPMyG22Sqp.json'"
        merch_keys = "tezos_account = '../../tezos-contract/pytezos-tests/sample_files/tz1bXwRiFvijKnZYUj9J53oYE3fFkMTWXqNx.json'"
    else:
        fatal_error("Not implemented yet: No tezos account for customer and merchant on '%s'" % network)

    if args.command == MERCH_SETUP:
        create_merchant_config(merch_db, merch_config, merch_keys, self_delay, confirmation_depth, url)
        info("Starting the merchant server...")
        zkchannel_merchant("run", config=merch_config, verbose=verbose)

    elif args.command == CUST_SETUP:
        create_customer_config(cust_db, cust_config, cust_keys, self_delay, confirmation_depth, url)
        info("Starting the customer watcher...")
        zkchannel_customer("watch", config=cust_config, verbose=verbose)

    elif args.command == SCENARIO:
        info("Running scenario: %s" % ', '.join(command_list))
        if network == SANDBOX:
            check_blockchain_maturity(url)
        t = TestScenario(
                cust_config, cust_db, 
                merch_config,
                config_path,
                channel_name, customer_deposit, 
                verbose
            )
        t.run_command_list(command_list)

    elif args.command == TEST_ALL:
        info("Running all test scenarios")
        if network == SANDBOX:
            check_blockchain_maturity(url)

        tests_to_run = []
        # Add tests for each closing method
        close_methods = (MUTUAL_CLOSE, CLOSE, EXPIRE)
        for close_method in close_methods:
            # Test each closing method for the following scenarios:
            # no payments, one payment, multiple payments, max payment
            tests_to_run.append([ESTABLISH, close_method])
            tests_to_run.append([ESTABLISH, PAY, close_method])
            tests_to_run.append([ESTABLISH, PAY, PAY, close_method])
            tests_to_run.append([ESTABLISH, PAY_ALL, close_method])

        # Dispute flow tests
        # Trigger 'dispute' by closing on revoked balances. 
        # Attempt closing on initial balance
        tests_to_run.append([ESTABLISH, STORE, PAY, RESTORE, CLOSE])
        # Attempt closing on non-initial balance
        tests_to_run.append([ESTABLISH, PAY, STORE, PAY, RESTORE, CLOSE])
        # Attempt closing on revoked state multiple states in the past
        tests_to_run.append([ESTABLISH, STORE, PAY, PAY, PAY, RESTORE, CLOSE])
        # Attempt closing on initial state after spending everything in the channel
        tests_to_run.append([ESTABLISH, STORE, PAY_ALL, RESTORE, CLOSE])

        info("The following scenarios will be tested:")
        pprint(tests_to_run)
        for i, test in enumerate(tests_to_run):
            info(f"Running {test}")
            channel_name = f"my-zkchannel-{i}"
            t = TestScenario(
                    cust_config, cust_db, 
                    merch_config,
                    config_path,
                    channel_name, customer_deposit, 
                    verbose
                )
            t.run_command_list(test)
        info("Done!")

    else:
        # list the available channels
        zkchannel_customer("list", config=cust_config, verbose=verbose)

main()
