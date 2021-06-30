#!/usr/bin/env python3

from serdesZ import serialize, deserialize
import json
import sys
import binascii

def decompress_point(p, is_G2=True, verbose=True):
    a = deserialize(binascii.unhexlify(p), is_G2)
    b = serialize(a, compressed=False)
    if verbose: print(b.hex())
    c = serialize(a, compressed=True)
    assert p == c.hex()
    return b.hex()

if __name__ == "__main__":
    value = sys.argv[1]
    type = sys.argv[2]
    if type == "G1":
        is_g2 = False
    elif type == "G2":
        is_g2 = True  
    dec_value = decompress_point(value, is_g2)
