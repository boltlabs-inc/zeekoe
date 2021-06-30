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

def add_funding(ci, amt):
    print("Adding funds ({})".format(amt))
    out = ci.addFunding().with_amount(amt).inject(_async=False)
    print("addFunding ophash: ", out['hash'])
    return out

def originate(cust_py, init_params, cust_funding, merch_funding):
    # Create initial storage for main zkchannel contract
    merch_ps_pk = init_params.get("merchant_ps_public_key")
    close_scalar_bytes = init_params.get("close_scalar_bytes")
    channel_id = init_params.get("channel_id")

    # Merchant's PS pubkey, used for verifying the merchant's signature in custClose.
    g2 = merch_ps_pk.get("g2")
    y2s = merch_ps_pk.get("y2s")
    x2 = merch_ps_pk.get("x2")

    initial_storage = {'cid': channel_id, 
    'close_flag': close_scalar_bytes,
    'context_string': "zkChannels mutual close",
    'custAddr': cust_addr, 
    'custBal':0, 
    'custFunding': cust_funding, 
    'custPk': cust_pubkey, 
    'delayExpiry': '1970-01-01T00:00:00Z', 
    'g2':g2,
    'merchAddr': merch_addr, 
    'merchBal': 0, 
    'merchFunding': merch_funding, 
    'merchPk': merch_pubkey, 
    'merchPk0': y2s[0],
    'merchPk1': y2s[1],
    'merchPk2': y2s[2],
    'merchPk3': y2s[3],
    'merchPk4': y2s[4],
    'merchPk5': x2,
    'revLock': '0x00', 
    'selfDelay': 3, 
    'status': 0}

    # Originate main zkchannel contract
    print("Originate main zkChannel contract")
    out = cust_py.origination(script=main_code.script(initial_storage=initial_storage)).autofill().sign().inject(_async=False)
    print("Originate zkChannel ophash: ", out['hash'])
    # Get address of main zkchannel contract
    opg = pytezos.shell.blocks[-20:].find_operation(out['hash'])
    main_id = opg['contents'][0]['metadata']['operation_result']['originated_contracts'][0]
    print("zkChannel contract address: ", main_id)
    return out, main_id

def cust_close(ci, cust_close_data):
    # Form cust close storage
    cs = cust_close_data.get("closing_signature")
    sigma1, sigma2 = cs.get("sigma1"), cs.get("sigma2")
    revocation_lock = cust_close_data.get("revocation_lock")
    # assumes it's already in the mutez
    cust_balance = str(cust_close_data.get("customer_balance"))
    merch_balance = str(cust_close_data.get("merchant_balance"))

    # close_storage = {
    #     "custBal": cust_balance,
    #     "merchBal": merch_balance,
    #     "revLock": revocation_lock,
    #     "s1": sigma1,
    #     "s2": sigma2
    # }

    print("Broadcasting Cust Close")
    #out = ci.custClose(close_storage).inject(_async=False)
    #print("Cust Close ophash: ", out['hash'])
    entrypoint = "custClose"
    cmd = 'ci.{e}(\"{custBal}\", \"{merchBal}\", \"{revLock}\", \"{s1}\", \"{s2}\").inject(_async=False)'.format(e=entrypoint, 
    custBal=cust_balance, merchBal=merch_balance, revLock=revocation_lock, s1=sigma1, s2=sigma2)
    out = eval(cmd)
    print("{} ophash: ".format(entrypoint), out['hash'])
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
    print("contract id: {}", contract_id)

    # Set the contract interfaces for cust
    cust_ci = cust_py.contract(contract_id)

    # add customer's balance to the contract using 'addFunding' entrypoint
    out = add_funding(cust_ci, cust_funding)
    feetracker.add_result('addFunding', out)

    return contract_id

def zkchannel_unilateral_close(feetracker, contract_id, cust_py, merch_py, cust_close_data):
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


# def scenario1(feetracker, cust_py, merch_py, cust_close_json):
#     '''
#     Scenario 1: Customer creates a single funded contract then initiates a unilateral closure.
#     Entrypoints tested: 'addFunding', 'custClose', 'custClaim'.
#     '''
#     print("Scenario 1: origination -> cust_funding -> cust_close -> cust_claim")

#     # A single-funded channel is originated where the customer's initial balances (in mutez) is:
#     cust_funding=30000000
#     merch_funding=0
#     out, main_id = originate(cust_py, cust_close_json, cust_funding, merch_funding)
#     feetracker.add_result('originate', out) # feetracker is used to track fees for benchmarking purposes 

#     # Set the contract interfaces for cust
#     cust_ci = cust_py.contract(main_id)

#     # add customer's balance to the contract using 'addFunding' entrypoint
#     out = add_funding(cust_ci, cust_funding)
#     feetracker.add_result('addFunding', out)

#     # customer initates unilateral closure using 'custClose' entrypoint. 
#     # The merchant's closing signature is included in 'cust_close_json'.
#     out = cust_close(cust_ci, cust_close_json)
#     feetracker.add_result('custClose', out)
#     out = entrypoint_no_args(cust_ci, 'custClaim')
#     feetracker.add_result('custClaim', out)

# def scenario2(feetracker, cust_py, merch_py, cust_close_json):
#     '''
#     Scenario 2: First the merchant initiates a unilateral closure, then the customer closes on an outdated state and is punished by the merchant.
#     Entrypoints tested: 'addFunding', 'expiry', 'custClose', 'merchDispute'.
#     '''
#     print("Scenario 2: origination -> cust_funding -> merch_funding -> expiry -> cust_close -> merch_dispute")
#     # A dual-funded channel is being created where the initial balances (in mutez) will be:
#     cust_funding=20000000
#     merch_funding=10000000
#     # The customer originates the custom zkchannel contract with initial storage arguments
#     out, main_id = originate(cust_py, cust_close_json, cust_funding, merch_funding)

#     # Set contract interfaces for cust and merch
#     cust_ci = cust_py.contract(main_id)
#     merch_ci = merch_py.contract(main_id)

#     # customer funds their side of the contract with the same amount as their initial balance
#     out = add_funding(cust_ci, cust_funding)

#     # At this point, the customer sends the contract (KT1) address to the merchant.
#     # After the customer's funding has reached 20 confirmations, the merchant funds their side of the channel.
#     out = add_funding(merch_ci, merch_funding)

#     # Merchant initiates closure by calling the 'expiry' entrypoint.
#     out = entrypoint_no_args(merch_ci, 'expiry')

#     # Customer initiates closure in response to 'expiry'.
#     out = cust_close(cust_ci, cust_close_json)

#     # Here, we simulate the customer closing on an old state. 
#     # The merchant views the contract storage and checks if 'rev_lock' has been seen before. 
#     # rev_lock has been seen, the merchant will punish the customer by calling 'merchDispute' and providing the revocation secret.
#     # If the secret is valid, the contract will give the total balance to the merchant.
#     rev_secret = add_hex_prefix(merch_close_json['rev_secret'])
#     out = merch_dispute(merch_ci, 'merchDispute', rev_secret)
#     feetracker.add_result('merchDispute', out)

# def scenario3(feetracker, cust_py, merch_py, cust_close_json):
#     '''
#     Scenario 3: Tests the 'reclaimFunding' and 'merchClaim' entrypoints.
#     'reclaimFunding' is to be used in a dual funded channel where only one party has deposited their funds and wishes to abort channel establishment.
#     'expiry' is used by the merchant to force the customer to close the channel within the delay period.
#     'merchClaim' is used by the merchant to claim the total channel balance if the customer fails to respond to the 'expiry' call.
#     Entrypoints tested: 'addFunding', 'reclaimFunding', 'expiry', 'merchClaim'.
#     '''
#     print("Scenario 3: origination -> cust_funding -> reclaim_funding -> cust_funding -> merch_funding -> expiry -> merch_claim")

#     cust_funding=20000000
#     merch_funding=10000000
#     out, main_id = originate(cust_py, cust_close_json, cust_funding, merch_funding)

#     # Set pytezos contract interfaces for the cust and merch
#     cust_ci = cust_py.contract(main_id)
#     merch_ci = merch_py.contract(main_id)

#     # customer funds their side of the contract with the same amount as their initial balance
#     out = add_funding(cust_ci, cust_funding)

#     # The customer reclaims their initial funding. 
#     # This entrypoint is used in the situation where the merchant aborts channel establishment and the customer wants to get their funds back.
#     out = entrypoint_no_args(cust_ci, 'reclaimFunding')
#     feetracker.add_result('reclaimFunding', out)

#     # customer and merchant fund their balances
#     out = add_funding(cust_ci, cust_funding)
#     out = add_funding(merch_ci, merch_funding)

#     out = entrypoint_no_args(merch_ci, 'expiry')
#     feetracker.add_result('expiry', out)

#     # If the customer has not broadcasted 'custClose' in response to 'expiry', the merchant can claim the full channel balance with 'merchClaim'.
#     out = entrypoint_no_args(merch_ci, 'merchClaim')
#     feetracker.add_result('merchClaim', out)


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
        main_id = zkchannel_establish(feetracker, cust_py, merch_py, establish_json)
        print("Contract ID: ", main_id)

    if args.cust_close:
        cust_close_json = read_json_file(cust_close_json_file)
        contract_id = args.contract_id
        if contract_id is None:
            sys.exit("[!] Need the contract id to proceed with cust close")
        zkchannel_unilateral_close(feetracker, contract_id, cust_py, merch_py, cust_close_json)

    #TODO: add merch expiry flow as well
    feetracker.print_fees()

    print("Tests finished!")
