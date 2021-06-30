#!/usr/bin/env python3

from serdesZ import serialize, deserialize
import json
import sys
import argparse
import binascii

def decompress_point(p, is_G2=True, verbose=True):
    a = deserialize(binascii.unhexlify(p), is_G2)
    b = serialize(a, compressed=False)
    if verbose: print(b.hex())
    c = serialize(a, compressed=True)
    assert p == c.hex()
    return b.hex()

def read_json_file(f):
    f = open(f)
    s = f.read()
    d = json.loads(s)
    f.close()
    return d

def add_hex_prefix(s):
    if s[:2] == "0x":
        return s
    return "0x" + s

def convert_to_little_endian(s):
    t = s
    if s[:2] == "0x":
        t = s[2:]
    return bytes.fromhex(t)[::-1].hex()

def convert_list_to_hex_string(data, field, type="", verbose=False, little_endian=False):
    field_raw = data.get(field)
    field_bytes = bytes(field_raw)
    if little_endian:
        data[field] = add_hex_prefix(convert_to_little_endian(field_bytes.hex()))
    else:
        if type in ["G1", "G2"]:
            data[field] = add_hex_prefix(decompress_point(field_bytes.hex(), type == "G2", verbose=verbose))
        else:
            data[field] = add_hex_prefix(field_bytes.hex())
    if verbose: print("{field} = {hex}".format(field=field, hex=data[field]))

def convert_vec_list_to_hex_string(data, field, type="", verbose=False):
    vec = []
    for r in data[field]:
        vec.append(add_hex_prefix(decompress_point(bytes(r).hex(), type == "G2", verbose=verbose)))
    data[field] = vec
    if verbose: print("{field} = {hex}".format(field=field, hex=data[field]))

def transform_establish_json_file(data, _verbose):
    original_data = dict(data)
    merchant_ps_public_key = original_data.get("merchant_ps_public_key")
    convert_list_to_hex_string(merchant_ps_public_key, "g1", "G1", verbose=_verbose)
    convert_list_to_hex_string(merchant_ps_public_key, "g2", "G2", verbose=_verbose)
    convert_list_to_hex_string(merchant_ps_public_key, "x2", "G2", verbose=_verbose)
    convert_vec_list_to_hex_string(merchant_ps_public_key, "y1s", "G1", verbose=_verbose)
    convert_vec_list_to_hex_string(merchant_ps_public_key, "y2s", "G2", verbose=_verbose)
    convert_list_to_hex_string(original_data, "channel_id", little_endian=False, verbose=_verbose)
    convert_list_to_hex_string(original_data, "close_scalar_bytes", little_endian=False, verbose=_verbose)
    return original_data

def transform_close_json_file(data, _verbose):
    original_data = dict(data)
    convert_list_to_hex_string(original_data, "channel_id", little_endian=True, verbose=_verbose)
    convert_list_to_hex_string(original_data, "revocation_lock", little_endian=True, verbose=_verbose)
    close_sig = original_data.get("closing_signature")
    convert_list_to_hex_string(close_sig, "sigma1", "G1", verbose=_verbose)
    convert_list_to_hex_string(close_sig, "sigma2", "G1", verbose=_verbose)
    original_data["closing_signature"] = close_sig
    return original_data

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="testing tool for formatting establish and close json for pytezos")
    parser.add_argument('--json', '-j', help="the json file", required=True)
    parser.add_argument('--out', '-o', help="output json file with decompressed group elements", required=True)
    parser.add_argument('--establish', '-e', help="originate the contract and fund the channel", action='store_true')
    parser.add_argument('--close', '-c', help="call the cust close entrypoint in the channel", action='store_true')
    parser.add_argument('--verbose', '-v', help='increase verbosity', action='store_true')
    args = parser.parse_args()

    if not args.establish and not args.close:
        sys.exit("Need to specify --establish or --close flag")

    json_data = read_json_file(args.json)
    if args.close:
        data = transform_close_json_file(json_data, args.verbose)
    if args.establish:
        data = transform_establish_json_file(json_data, args.verbose)   

    decompressed_data_str = json.dumps(data, indent=4)
    if args.verbose:
        print(decompressed_data_str)
    f = open(args.out, "w")
    f.write(decompressed_data_str)
    f.close()
