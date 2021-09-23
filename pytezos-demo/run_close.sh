#!/bin/bash

TEZOS_CONTRACT=$1
if [[ $TEZOS_CONTRACT = "" ]]; then
   echo "[!] Missing path to tezos_contract repo dir as first arg"
   exit -1
fi

KEY=$TEZOS_CONTRACT/pytezos-tests/sample_files
JSON=$2
if [[ $JSON = "" ]]; then
   echo "[!] Missing close json file as second arg"
   exit -1
fi

CONTRACT_ID=$3
if [[ $CONTRACT_ID = "" ]]; then
   echo "[!] Missing contract id to call close"
   exit -1
fi

./convert_format.sh $JSON close
python3 zkchannel_pytezos_mgr.py --contract=$TEZOS_CONTRACT/zkchannels-contract/zkchannel_contract.tz --contract-id=$CONTRACT_ID \
	--cust=$KEY/tz1iKxZpa5x1grZyN2Uw9gERXJJPMyG22Sqp.json --merch=$KEY/tz1bXwRiFvijKnZYUj9J53oYE3fFkMTWXqNx.json \
	--cust-close out.$JSON

