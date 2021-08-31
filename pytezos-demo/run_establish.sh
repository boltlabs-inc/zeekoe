#!/bin/bash

TEZOS_CONTRACT=$1
if [[ $TEZOS_CONTRACT = "" ]]; then
   echo "[!] Missing path to tezos_contract repo dir as first arg"
   exit -1
fi

JSON=$2
if [[ $JSON = "" ]]; then
   echo "[!] Missing establish json file as second arg"
   exit -1
fi

KEY=$TEZOS_CONTRACT/pytezos-tests/sample_files

./convert_format.sh $JSON establish
python3 zkchannel_pytezos_mgr.py --contract=$TEZOS_CONTRACT/zkchannels-contract/zkchannel_contract.tz --cust=$KEY/tz1S6eSPZVQzHyPF2bRKhSKZhDZZSikB3e51.json --merch=$KEY/tz1VcYZwxQoyxfjhpNiRkdCUe5rzs53LMev6.json --establish out.$JSON

