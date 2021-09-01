# Example usage:
# python3 zkchannel_pytezos_mgr.py --contract=zkchannel_contract.tz --cust=tz1S6eSPZVQzHyPF2bRKhSKZhDZZSikB3e51.json --merch=tz1VcYZwxQoyxfjhpNiRkdCUe5rzs53LMev6.json --custclose=cust_close.json --merchclose=merch_close.json 

import argparse
from pprint import pprint
from pytezos import pytezos
from pytezos import Contract
from pytezos import ContractInterface
import json
import sys

def read_json_file(json_file):
    f = open(json_file)
    s = f.read()
    f.close()
    return json.loads(s)

def convert_mt_to_tez(balance):
    return str(int(balance) /1000000)

class FeeTracker:
    def __init__(self):
        self.fees = []
    
    def add_result(self, op_name, result):
        """Add the fees of the fees from operation result to self.fees"""
        fee = int(result['contents'][0]['fee'])
        storage_bytes = int(result['contents'][0]['storage_limit'])
        storage_cost = int(storage_bytes) * 250 # 250 mutez per storage_bytes byte on edo
        gas = int(result['contents'][0]['gas_limit'])
        total_cost = fee + storage_cost
        fee = {"total_cost":total_cost, "fee":fee, "storage_bytes":storage_bytes, "storage_cost":storage_cost, "gas":gas}
        self.fees.append({op_name:fee})

    def print_fees(self):
        pprint(self.fees)

def add_cust_funding(ci, amt):
    print("Adding funds ({})".format(amt))
    out = ci.addCustFunding().with_amount(amt).send(min_confirmations=1)
    print("Add Cust Funding ophash: ", out['hash'])
    return out

def originate(cust_py, init_params, cust_funding, merch_funding):
    # Create initial storage for main zkchannel contract
    merch_ps_pk = init_params.get("merchant_ps_public_key")
    close_scalar_bytes = init_params.get("close_scalar_bytes")
    channel_id = init_params.get("channel_id")

    # Merchant's PS pubkey, used for verifying the merchant's signature in custClose.
    merch_g2 = merch_ps_pk.get("g2")
    merch_y2s = merch_ps_pk.get("y2s")
    merch_x2 = merch_ps_pk.get("x2")
    
    initial_storage = {"cid": channel_id, 
    "close_scalar": close_scalar_bytes,
    "context_string": "zkChannels mutual close",
    "customer_address": cust_addr, 
    "customer_balance": cust_funding,  
    "customer_public_key": cust_pubkey, 
    "delay_expiry": "1970-01-01T00:00:00Z", 
    "g2": merch_g2,
    "merchant_address": merch_addr, 
    "merchant_balance": merch_funding, 
    "merchant_public_key": merch_pubkey, 
    "y2s_0": merch_y2s[0],
    "y2s_1": merch_y2s[1],
    "y2s_2": merch_y2s[2],
    "y2s_3": merch_y2s[3],
    "y2s_4": merch_y2s[4],
    "x2": merch_x2,
    "revocation_lock": "0x00", 
    "self_delay": 172800, 
    "status": 0}

    # Originate main zkchannel contract
    print("Originate main zkChannel contract")
    out = cust_py.origination(script=main_code.script(initial_storage=initial_storage)).autofill().sign().inject(_async=False)
    print("Originate zkChannel ophash: ", out['hash'])
    # Get address of main zkchannel contract
    opg = pytezos.shell.blocks[-20:].find_operation(out['hash'])
    contract_id = opg['contents'][0]['metadata']['operation_result']['originated_contracts'][0]
    print("zkChannel contract address: ", contract_id)
    return out, contract_id

def cust_close(ci, cust_close_data):
    # Form cust close storage
    cs = cust_close_data.get("closing_signature")
    sigma1, sigma2 = cs.get("sigma1"), cs.get("sigma2")
    revocation_lock = cust_close_data.get("revocation_lock")
    # assumes it's already in the mutez
    cust_balance = convert_mt_to_tez(cust_close_data.get("customer_balance"))
    merch_balance = convert_mt_to_tez(cust_close_data.get("merchant_balance"))

    close_storage = {
        "custBal": cust_balance,
        "merchBal": merch_balance,
        "revLock": revocation_lock,
        "s1": sigma1,
        "s2": sigma2
    }

    print("Broadcasting Cust Close: %s" % close_storage)
    out = ci.custClose(close_storage).inject(_async=False)
    print("Cust Close ophash: ", out['hash'])
    return out

def merch_dispute(ci, entrypoint, rev_secret):
    print('Broadcasting {}'.format(entrypoint))
    cmd = 'ci.{e}(\"{r}\").inject(_async=False)'.format(e=entrypoint, r=rev_secret)
    out = eval(cmd)
    print("{} ophash: ".format(entrypoint), out['hash'])
    return out

def entrypoint_no_args(ci, entrypoint):
    print('Broadcasting {}'.format(entrypoint))
    cmd = 'ci.{}().inject(_async=False)'.format(entrypoint)
    out = eval(cmd)
    print("{} ophash: ".format(entrypoint), out['hash'])
    return out

def zkchannel_establish(feetracker, cust_py, merch_py, establish_params):
    '''
    Customer originates a single funded contract.
    Entrypoints tested: 'addFunding'
    '''
    cust_funding=establish_json.get("customer_deposit")
    merch_funding=establish_json.get("merchant_deposit")
    out, contract_id = originate(cust_py, establish_params, cust_funding, merch_funding)
    feetracker.add_result('originate', out) # feetracker is used to track fees for benchmarking purposes 
    print("Contract ID: ", contract_id)

    # Set the contract interfaces for cust
    cust_ci = cust_py.contract(contract_id)

    # add customer's balance to the contract using 'addFunding' entrypoint
    out = add_cust_funding(cust_ci, cust_funding)
    feetracker.add_result('addFunding', out)

    return contract_id

def zkchannel_unilateral_close(feetracker, contract_id, cust_py, _merch_py, cust_close_data):
    '''
    Customer or Merchant can proceed with broadcasting 
    closing signature on final state of the channel.
    Entrypoints tested: 'custClose', 'custClaim'
    '''
    # Set the contract interfaces for cust
    print("Getting handle to the contract: '%s'" % contract_id)
    cust_ci = cust_py.contract(contract_id)

    out = cust_close(cust_ci, cust_close_data)
    feetracker.add_result('custClose', out)

    out = entrypoint_no_args(cust_ci, 'custClaim')
    feetracker.add_result('custClaim', out)

    return 

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--shell", "-n", required=False, help="the address to connect to edo2net", default = "https://rpc.tzkt.io/edo2net/")
    parser.add_argument("--contract", "-z", required=True, help="zkchannels michelson contract")
    parser.add_argument("--contract-id", help="specify the contract id")
    parser.add_argument("--cust", "-c", required=True, help="customer's testnet account json file")
    parser.add_argument("--merch", "-m", required=True, help="merchant's testnet account json file")
    parser.add_argument("--establish", "-e", help="Filename (with path) to <chanid>.establish.json file created by zeekoe")
    parser.add_argument("--cust-close", "-cc", help="Filename (with path) to the <chanid>.close.json file created by zeekoe")
    # parser.add_argument("--merch_close", "-mc", help="Enter the filename (with path) to the merch_expiry.json file created by zeekoe")
    args = parser.parse_args()

    if args.shell:
        pytezos = pytezos.using(shell=args.shell)
    print("Connecting to edo2net via: " + args.shell)
    cust_acc = args.cust
    merch_acc = args.merch
    establish_json_file = args.establish
    cust_close_json_file = args.cust_close
    # merch_close_file = args.merch_close

    # Set customer and merch pytezos interfaces
    cust_py = pytezos.using(key=cust_acc)
    cust_addr = read_json_file(cust_acc)['pkh']
    merch_py = pytezos.using(key=merch_acc)
    merch_addr = read_json_file(merch_acc)['pkh']
    # merch_close_json = read_json_file(merch_close_file)

    # load zchannel contracts
    main_code = ContractInterface.from_file(args.contract)

    # Activate cust and merch testnet accounts
    if args.establish:
        try:
            print("Activating cust account")
            cust_py.activate_account().fill().sign().inject()
        except:
            print("Cust account already activated")

        try:
            print("Revealing cust pubkey")
            out = cust_py.reveal().autofill().sign().inject()
        except:
            pass
    cust_pubkey = cust_py.key.public_key()

    if args.establish:
        try:
            print("Activating merch account")
            merch_py.activate_account().fill().sign().inject()
        except: 
            print("Merch account already activated")

        try:
            print("Revealing merch pubkey")
            out = merch_py.reveal().autofill().sign().inject()
        except:
            pass
    merch_pubkey = merch_py.key.public_key()

    feetracker = FeeTracker()
    if args.establish:
        establish_json = read_json_file(establish_json_file)
        contract_id = zkchannel_establish(feetracker, cust_py, merch_py, establish_json)
        print("Contract ID (confirmed): ", contract_id)

    if args.cust_close:
        cust_close_json = read_json_file(cust_close_json_file)
        contract_id = args.contract_id
        if contract_id is None:
            sys.exit("[!] Need the contract id to proceed with cust close")
        zkchannel_unilateral_close(feetracker, contract_id, cust_py, merch_py, cust_close_json)

    #TODO: add merch expiry flow as well
    feetracker.print_fees()

    print("Tests finished!")
