# To install requests library, run the following:
# pip install requests
#
# To setup the merchant's sandbox config and start the merchant server, run the following:
# $: python3 test-zeekoe.py merch-setup --url "http://localhost:20000" -v
# 
# Then setup the customer's sandbox config and start the chain watcher, run the following:
# $: python3 test-zeekoe.py cust-setup --url "http://localhost:20000" -v
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

# The minimum blockchain level to be able to run tests. Operations need to reference a block up to 
# 60 blocks from the head. Setting this minimum level avoids running into errors caused by the 
# blockchain not having enough blocks. 
MIN_BLOCKCHAIN_LEVEL = 60

def info(msg):
    print("%s[+] %s%s" % (GREEN, msg, NC))

def log(msg, debug=True):
    if debug: print("%s%s%s" % (BBlack, msg, NC))

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
    while True:
        try:
            output = process.stdout.readline()
            if process.poll() is not None:
                break
            if output:
                log("-> %s" % output.strip().decode('utf-8'), verbose)
        except KeyboardInterrupt:
            process.terminate()
    rc = process.poll()
    error = process.stderr.readline()
    log("-> %s" % error.strip().decode('utf-8'), verbose)
    return rc

def start_merchant_server(merch_config, verbose):
    info("Starting the merchant server...")
    cmd = ["./target/debug/zkchannel", "merchant", "--config", merch_config, "run"]
    return run_command(cmd, verbose)

def start_customer_watcher(cust_config, verbose):
    info("Starting the customer watcher...")
    cmd = ["./target/debug/zkchannel", "customer", "--config", cust_config, "watch"]
    return run_command(cmd, verbose)

def create_new_channel(cust_config, channel_name, initial_deposit, verbose):
    info("Creating a new zkchannel: %s" % channel_name)
    initial_deposit = "{amount} XTZ".format(amount=str(initial_deposit))
    cmd = ["./target/debug/zkchannel", "customer", "--config", cust_config, "establish", "zkchannel://localhost", "--deposit", initial_deposit, "--label", channel_name]
    return run_command(cmd, verbose)

def make_payment(cust_config, channel_name, pay_amount, verbose):
    info("Making a %s payment on zkchannel: %s" % (pay_amount, channel_name))
    payment = "{amount} XTZ".format(amount=str(pay_amount))
    cmd = ["./target/debug/zkchannel", "customer", "--config", cust_config, "pay", channel_name, payment]
    return run_command(cmd, verbose)

def close_channel(cust_config, channel_name, verbose):
    info("Initiate closing on the zkchannel: %s" % channel_name)
    cmd = ["./target/debug/zkchannel", "customer", "--config", cust_config, "close", "--force", channel_name]
    return run_command(cmd, verbose)

def list_channels(cust_config):
    info("List channels...")
    cmd = ["./target/debug/zkchannel", "customer", "--config", cust_config, "list"]
    return run_command(cmd, True)

def expire_channel(merch_config, channel_id, verbose):
    info("Initiate expiry on the channel id: %s" % channel_id)
    cmd = ["./target/debug/zkchannel", "merchant", "--config", merch_config, "close", "--channel", channel_id]
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
            dev_path,  
            channel_name, customer_deposit, 
            verbose
        ):
        self.cust_config = cust_config
        self.merch_config = merch_config
        self.dev_path = dev_path
        self.cust_db = cust_db
        self.temp_path = os.path.join(dev_path, "temp")
        self.channel_path = os.path.join(self.temp_path, f"{channel_name}")
        self.channel_name = channel_name
        self.customer_deposit = float(customer_deposit)
        self.balance_remaining = float(customer_deposit)
        self.verbose = verbose

         # Create temporary directory to store revoked customer state when testing dispute scenarios
        if not os.path.isdir(self.temp_path):
            os.mkdir(self.temp_path)

    def establish(self):
        create_new_channel(self.cust_config, self.channel_name, self.customer_deposit, self.verbose)

    def pay(self):
        max_pay_amount = self.balance_remaining / 2 # save money for future payments
        pay_amount = round(random.uniform(0, max_pay_amount), 4)
        make_payment(self.cust_config, self.channel_name, pay_amount, self.verbose)
        self.balance_remaining -= pay_amount

    def pay_all(self):
        pay_amount = self.balance_remaining
        make_payment(self.cust_config, self.channel_name, pay_amount, self.verbose)
        self.balance_remaining = 0

    def close(self):
        close_channel(self.cust_config, self.channel_name, self.verbose)

    def transfer_db_files(self, src, dst, db_name):
        """transfer all db files '-shm' and '-wal' """
        db_path = os.path.join(src, db_name)
        for file in glob.glob(db_path + '*'):
            db_name = os.path.basename(file)
            # set path for destination db file
            new_file = os.path.join(dst, db_name)
            shutil.copyfile(file, new_file)

    def store_state(self):
        log("Storing customer state with remaining balance of %s" % self.balance_remaining)
        # Create temporary directory to store revoked customer state when testing dispute scenarios
        if not os.path.isdir(self.channel_path):
            os.mkdir(self.channel_path)
        self.transfer_db_files(src = self.dev_path, dst = self.channel_path, db_name = self.cust_db)

    def restore_state(self):
        log("Restoring customer state")
        self.transfer_db_files(src = self.channel_path, dst = self.dev_path, db_name = self.cust_db)
        
    def expire(self):
        # TODO: Get channel_id from a channel_name
        list_channels(self.cust_config)
        channel_id = input("Enter the channel_id to be expired\n")
        expire_channel(self.merch_config, channel_id, self.verbose)

    def run_command_list(self, command_list):
        for command in command_list:
            if command == "establish":
                self.establish()
            elif command == "pay":
                self.pay()
            elif command == "pay_all":
                self.pay_all()
            elif command == "close":
                self.close()
            elif command == "store":
                self.store_state()
            elif command == "restore":
                self.restore_state()
            elif command == "expire":
                self.expire()
            else:
                fatal_error(f"{command} not a recognized command.")


COMMANDS = ["list", "merch-setup", "cust-setup", "scenario"]
def main():
    parser = argparse.ArgumentParser(formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("command", help="", nargs="?", default="list")
    parser.add_argument("--path", help="path to create configs", default="./dev")
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
    dev_path = args.path
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

    cust_config = os.path.join(dev_path, f"Customer-{network}.toml")
    cust_db = f"customer-{network}.db"
    merch_config = os.path.join(dev_path, f"Merchant-{network}.toml")
    merch_db = f"merchant-{network}.db"
    channel_name = f"my-zkchannel-{str(channel_count)}"

    if network == SANDBOX:
        cust_keys = "tezos_account = { alias = \"alice\" }"
        merch_keys = "tezos_account = { alias = \"bob\" }"        
    elif network == TESTNET:
        cust_keys = "tezos_account = '../tezos-contract/pytezos-tests/sample_files/tz1iKxZpa5x1grZyN2Uw9gERXJJPMyG22Sqp.json'"
        merch_keys = "tezos_account = '../tezos-contract/pytezos-tests/sample_files/tz1bXwRiFvijKnZYUj9J53oYE3fFkMTWXqNx.json'"
    else:
        fatal_error("Not implemented yet: No tezos account for customer and merchant on '%s'" % network)


    if args.command == MERCH_SETUP:
        create_merchant_config(merch_db, merch_config, merch_keys, self_delay, confirmation_depth, url)
        start_merchant_server(merch_config, verbose)

    elif args.command == CUST_SETUP:
        create_customer_config(cust_db, cust_config, cust_keys, self_delay, confirmation_depth, url)
        start_customer_watcher(cust_config, verbose)

    elif args.command == SCENARIO:
        info("Running scenario: %s" % ', '.join(command_list))
        if network == SANDBOX:
            check_blockchain_maturity(url)
        t = TestScenario(
                cust_config, cust_db, 
                merch_config,
                dev_path,
                channel_name, customer_deposit, 
                verbose
            )
        t.run_command_list(command_list)
    else:
        # list the available channels
        list_channels(cust_config)

main()
