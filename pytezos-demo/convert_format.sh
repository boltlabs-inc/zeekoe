#!/bin/bash

FILE=$1
if [[ $FILE = "" ]]; then
   echo "[!] Missing json file as first arg"
   exit -1
fi

MODE=$2
if [[ $MODE = "establish" ]]; then
   python3 testing-tool/format_output.py --establish --json $FILE --out out.$FILE 
   exit 0
elif [[ $MODE = "close" ]]; then
   python3 testing-tool/format_output.py --close --json $FILE --out out.$FILE 
   exit 0
else
   echo "[!] Invalid second arg: 'establish' or 'close'"
   exit -1
fi

