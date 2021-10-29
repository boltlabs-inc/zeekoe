#!/bin/sh

pip install requests
python3 test-zeekoe.py merch-setup --url "http://localhost:20000" -v
python3 test-zeekoe.py cust-setup --url "http://localhost:20000" -v
python3 test-zeekoe.py scenario --channel 1 -v --command-list establish pay pay pay_all close
python3 test-zeekoe.py scenario --channel 1 -v --command-list establish pay store pay restore close