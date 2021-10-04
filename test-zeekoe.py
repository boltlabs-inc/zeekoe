
#
# To setup the sandbox configs and start the merchant server, run the following:
# $: python3 test-zeekoe.py setup --url "http://localhost:20000" -v
# 
# Then test the life cycle of a few channels (ideally in parallel): establish a channel, make a payment and run cust close
# $: python3 test-zeekoe.py scenario --channel 1 --num-payments 5 -v
# $: python3 test-zeekoe.py scenario --channel 2 --num-payments 7 -v
#
# List the channels
# $: python3 test-zeekoe.py list
#

import argparse
import json
import subprocess
import sys
import random

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'
BBlack='\033[1;30m'

TESTNET = "testnet"
SANDBOX = "sandbox"

MERCH_SETUP = "merch-setup"
CUST_SETUP = "cust-setup"
SCENARIO = "scenario"

def info(msg):
    print("%s[+] %s%s" % (GREEN, msg, NC))

def log(msg, debug=True):
    if debug: print("%s%s%s" % (BBlack, msg, NC))

def fatal_error(msg):
    print("%sERROR:%s %s%s%s" % (BBlack, NC, RED, msg, NC))
    sys.exit(-1)

def create_merchant_config(merch_db, merch_config, merch_account_keys, self_delay, url_path, verbose=False):
    config_contents = """
database = {{ sqlite = "{merchant_db}" }}
{tezos_account}
tezos_uri = "{url}"
self_delay = {self_delay}

[[service]]
address = "::1"
private_key = "localhost.key"
certificate = "localhost.crt"

[[service]]
address = "127.0.0.1"
private_key = "localhost.key"
certificate = "localhost.crt"    
    """.format(merchant_db=merch_db, tezos_account=merch_account_keys, self_delay=self_delay, url=url_path)
    f = open(merch_config, "w")
    f.write(config_contents)
    f.close()
    info("-> Created merchant config: %s" % merch_config)
    if verbose:
        print("============")
        print(config_contents)
        print("============")
    return
    
def create_customer_config(cust_db, cust_config, cust_account_keys, self_delay, url_path, verbose=False):
    config_contents = """
database = {{ sqlite = "{customer_db}" }}
trust_certificate = "localhost.crt"
{tezos_account}
tezos_uri = "{url}"
self_delay = {self_delay}
    """.format(customer_db=cust_db, tezos_account=cust_account_keys, self_delay=self_delay, url=url_path)
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

def scenario_dispute_customer_close(config, channel_name, verbose):
    # TODO: take necessary steps to close on old state
    # TODO: then force close as usual
    pass

def scenario_close_with_expiry(config, channel_name, verbose):
    # TODO: initiate merch expiry
    # TODO: then customer should detect and respond with cust close
    pass

class TestScenario():
    def __init__(self, cust_config, channel_name, customer_deposit, verbose):
        self.cust_config = cust_config
        self.channel_name = channel_name
        self.customer_deposit = float(customer_deposit)
        self.balance_remaining = float(customer_deposit)
        self.verbose = verbose
    
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
            else:
                fatal_error(f"{command} not a recognized command.")


COMMANDS = ["list", "merch-setup", "cust-setup", "scenario"]
def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", help="", nargs="?", default="list")
    parser.add_argument("--path", help="path to create configs", default="./dev")
    parser.add_argument("--network", help="select the type of network", default="sandbox")
    parser.add_argument("--self-delay", "-t", type=int, help="self-delay for closing transactions", default="1")
    parser.add_argument("--url", "-u", help="url for tezos network", default="http://localhost:20000")
    parser.add_argument("--amount", "-a", help="starting balance for each channel", default="10")
    parser.add_argument("--verbose", "-v", help="increase output verbosity", action="store_true")
    parser.add_argument("--channel", type=int, help="desired starting channel counter", default="1")
    parser.add_argument('--command-list','-c', nargs='+', help='commands to be tested, e.g. establish pay close')

    args = parser.parse_args()

    if args.command not in COMMANDS:
        fatal_error("'%s' not a recognized command. Here are the options: %s" % (args.command, COMMANDS))
    
    verbose = args.verbose
    dev_path = args.path
    url = args.url.lower()
    network = args.network.lower()

    self_delay = args.self_delay
    customer_deposit = args.amount
    channel_count = args.channel
    command_list = args.command_list

    if int(channel_count) <= 0:
        fatal_error("Expected '--channel' to be > 0")

    if network not in [SANDBOX, TESTNET]:
        fatal_error("Specified invalid 'network' argument. Values: '%s' or '%s'" % (SANDBOX, TESTNET))

    cust_config = "{path}/Customer-{network}.toml".format(path=dev_path, network=network)
    cust_db = "customer-{network}.db".format(network=network)
    merch_config = "{path}/Merchant-{network}.toml".format(path=dev_path, network=network)
    merch_db = "merchant-{network}.db".format(network=network)
    channel_name = "my-zkchannel-{count}".format(count=str(channel_count))

    if network == SANDBOX:
        cust_keys = "tezos_account = { alias = \"alice\" }"

        merch_keys = "tezos_account = { alias = \"bob\" }"
    else:
        fatal_error("Not implemented yet: No tezos account for customer and merchant on '%s'" % network)

    if args.command == MERCH_SETUP:
        create_merchant_config(merch_db, merch_config, merch_keys, self_delay, url)
        start_merchant_server(merch_config, verbose)

    elif args.command == CUST_SETUP:
        create_customer_config(cust_db, cust_config, cust_keys, self_delay, url)
        start_customer_watcher(cust_config, verbose)

    elif args.command == SCENARIO:
        info("Running scenario: %s" % ', '.join(command_list))
        t = TestScenario(cust_config, channel_name, customer_deposit, verbose)
        t.run_command_list(command_list)
    else:
        # list the available channels
        list_channels(cust_config)

main()
