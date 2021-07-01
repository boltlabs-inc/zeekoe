Demo
====

This demo requires that you have pytezos installed as follows:

    pip install pytezos

Also, we require that you clone the tezos-contract repo here:

    git clone https://github.com/boltlabs-inc/tezos-contract.git


```bash
$ ./target/debug/zkchannel merchant --config "./dev/Merchant.toml" run
serving on: [::1]:2611
```

Establish the channel as follows:

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" \
    establish "zkchannel://localhost" \
    --label "my-first-zkchannel" \
    --deposit "5 XTZ"
Successfully established new channel with label "my-first-zkchannel"
Establishment data written to "5f0b6efabc46808589acc4ffcfa9e9c8412cc097e45d523463da557d2c675c67.establish.json"
```

Separately, run the pytezos script to originate and fund the contract (we include some funded accounts in the tezos-contract repo).

```bash
$ ./run_establish.sh /full-path/to/tezos-contract 5f0b6efabc46808589acc4ffcfa9e9c8412cc097e45d523463da557d2c675c67.establish.json

Connecting to edo2net via: https://rpc.tzkt.io/edo2net/
Activating cust account
Cust account already activated
Revealing cust pubkey
Activating merch account
Merch account already activated
Revealing merch pubkey
Originate main zkChannel contract
Wait 3 seconds until block BLD2AH1enw1cNXZvdcU6CoYRNqBet7kvAjxgiPzNUkvUZNm5x9o is finalized
Originate zkChannel ophash:  oojmM18VzvzjjbRntB8rCn2bu6jrCFZKMwgBk3x9XpwsB4ZXTEF
zkChannel contract address:  KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE
contract id: {} KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE
Adding funds (5000000)
Wait 18 seconds until block BM4b6PfvaEoyoQ53sSExp43pJZqr9z1w1EWSsooE7jjt4BiwNjG is finalized
addFunding ophash:  oo8FUY2UpoT1enibdaCphLkvfiLjpAJ3nmrcqWpVQZTmvfy3KbJ
Contract ID:  KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE
[{'originate': {'fee': 10879,
                'gas': 32278,
                'storage_bytes': 7649,
                'storage_cost': 1912250,
                'total_cost': 1923129}},
 {'addFunding': {'fee': 3585,
                 'gas': 33098,
                 'storage_bytes': 103,
                 'storage_cost': 25750,
                 'total_cost': 29335}}]
Tests finished!
```

We can see the contract operations here: https://edo2net.tzkt.io/KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE/operations/

We will need to provide the contract id later during channel closing. Now, when we list our channels, we can see that we have an open channel with 5 XTZ available to spend.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬───────┬──────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State ┆ Balance  ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪═══════╪══════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ ready ┆ 5.00 XTZ ┆ 0.00 XTZ   ┆ Xwtu+rxGgIWJrMT/z6npyEEswJfkXVI0Y9pVfSxnXGc= │
└────────────────────┴───────┴──────────┴────────────┴──────────────────────────────────────────────┘
```

And, on the merchant's side, a channel with the same ID has been established also.

```bash
$ ./target/debug/zkchannel merchant --config "./dev/Merchant.toml" list
┌──────────────────────────────────────────────┬────────┐
│ Channel ID                                   ┆ Status │
╞══════════════════════════════════════════════╪════════╡
│ Xwtu+rxGgIWJrMT/z6npyEEswJfkXVI0Y9pVfSxnXGc= ┆ active │
└──────────────────────────────────────────────┴────────┘
```

Now, we can make a few payments on this channel, in this case in the amount of 0.4 XTZ and 0.6 ZTZ.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" \
    pay "my-first-zkchannel" "0.4 XTZ"
```

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" \
    pay "my-first-zkchannel" "0.6 XTZ"
```

We can then check the balances in our channels again to confirm that the payments went through.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬───────┬───────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State ┆ Balance   ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪═══════╪═══════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ ready ┆ 4 XTZ     ┆ 1 XTZ      ┆ Xwtu+rxGgIWJrMT/z6npyEEswJfkXVI0Y9pVfSxnXGc= │
└────────────────────┴───────┴───────────┴────────────┴──────────────────────────────────────────────┘
```

Let's initiate close on the channel as follows:

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" close --force my-first-zkchannel
Closing data written to "5f0b6efabc46808589acc4ffcfa9e9c8412cc097e45d523463da557d2c675c67.close.json"
```

Now we can use pytezos again to broadcast on chain:

```bash
$ ./run_close.sh /full-path/to/tezos-contract/ 5f0b6efabc46808589acc4ffcfa9e9c8412cc097e45d523463da557d2c675c67.close.json KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE
Connecting to edo2net via: https://rpc.tzkt.io/edo2net/
Getting handle to the contract: 'KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE'
Broadcasting Cust Close: {'custBal': '4.0', 'merchBal': '1.0', 'revLock': '0x7723ecf912ca83f8c637e7341699dad476ba971506cbf5f6bdaaac313b761c2f', 's1': '0x1189f6f8bb0dc1c6d34abb4a00e9d990d1dd62a019bdbedf95c3d51b9b13bf5a38edb316f990c4142f5cc8ad6a14074a18c36110d08d3543d333f6f9c9fe42dc580774cce2f3d3d3e0eb498486cf2617477929e980faf9dc89be569b2b46e7cf', 's2': '0x101cae6b21d198c69532944c3fd06af167ccc256d3c27c4eca5ac501ce928d8c30467f549e8f4a8c82733943e06bd9290a12c39ddd1dc362b48e77a1fb629f3655a87b6a4d499183fc768717bf18666bb065825b8f06e72c40b68c8307a5e630'}
...
```

If the operation is successful, we should see a pending contract operations here: https://edo2net.tzkt.io/KT1Dc9vADeBVGzAEQytUD3H1MtgPUUMRCkNE/operations/.

```bash
$ ./target/debug/zkchannel customer --config "./dev/Customer.toml" list
┌────────────────────┬────────┬─────────┬────────────┬──────────────────────────────────────────────┐
│ Label              ┆ State  ┆ Balance ┆ Max Refund ┆ Channel ID                                   │
╞════════════════════╪════════╪═════════╪════════════╪══════════════════════════════════════════════╡
│ my-first-zkchannel ┆ closed ┆ 4 XTZ   ┆ 1 XTZ      ┆ Xwtu+rxGgIWJrMT/z6npyEEswJfkXVI0Y9pVfSxnXGc= │
└────────────────────┴────────┴─────────┴────────────┴──────────────────────────────────────────────┘
```

The merchant server may now be stopped by pressing ^C.
