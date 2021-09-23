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
python3 zkchannel_pytezos_mgr.py --contract=$TEZOS_CONTRACT/zkchannels-contract/zkchannel_contract.tz --cust=$KEY/tz1iKxZpa5x1grZyN2Uw9gERXJJPMyG22Sqp.json --merch=$KEY/tz1bXwRiFvijKnZYUj9J53oYE3fFkMTWXqNx.json --establish out.$JSON

