# Contributing to Zeekoe: a Project Map

Thanks for your interest in contributing to Zeekoe! This project is large, so it could be a little
intimidating to get started. This document is a map of the territory to help you get started.

If you find bugs, please report them using the [issue
tracker](https://github.com/boltlabs-inc/zeekoe/issues/new), or create a pull request. Thanks!

## Three functional agents and their binaries

Zeekoe is comprised of three agents:

- the **merchant server**
- the **customer client**
- the **customer daemon**

These three agents communicate with one another to enact the [zkChannels
protocol](https://github.com/boltlabs-inc/zkchannels-spec). A merchant who wishes to accept payments
via zkChannels runs the **merchant server**, awaiting connections from customers who invoke the
**customer client** to make payments.

Additionally, a merchant may _optionally_ configure the merchant server to call out to an **external
approver** service which they themselves define, which provides a policy that determines when to
allow or reject payments. The external approver is not a part of the Zeekoe core application; it is
defined by the merchant who wishes to integrate zkChannels into a larger application flow.

When you build Zeekoe, several binaries will be created:

- `zkchannel-customer`: the customer client and daemon
- `zkchannel-merchant`: the merchant server
- `zkchannel`: a universal binary able to act as customer client and daemon, and as merchant server

Running `zkchannel customer` with some following command line options is equivalent to running the
`zkchannel-customer` binary with those options; likewise, running `zkchannel merchant` with some
following command line options is equivalent to running the `zkchannel-merchant` binary with those
options. Unless there is a specific reason to want to reduce binary size as much as possible, there
is no reason not to distribute and use the universal `zkchannel` binary.

## Crate module structure

The application's support library is broken up into functionalities:

- [Configuration](#configuration)
- [Command-line interface](#command-line-interface)
- [Database](#database)
- [Escrow](#escrow)
- [Protocol](#protocol)
- [Transport](#transport)
- [Amount](#amount)

These are each defined in separate modules, with submodules if necessary to define differences
between the customer and merchant implementation of these functionalities. They are then exposed as
a library crate to the binary crates, which consume this library interface to build their
functionality. The library should _not_ be considered a stable API; it is an internal implementation
detail.

Notably, the module structure exposed by the library is not the same as the file structure on disk.
While the project groups related functionalities together into the same directories and clusters
merchant and customer functionality under those directories, the exported module structure inverts
those orders. For example, the module defined in `src/database/customer.rs` is exposed from the
library as `zeekoe::customer::database`.

The binaries built using this library are broken up by the subprotocol of the zkChannels protocol
they participate in:

- Establish
- Pay
- Close
- Get Parameters (merchant only)

On the customer side, each of these is associated with a specific _customer command_ that the human
customer will invoke to cause the action to occur. On the merchant side, each is associated with a
specific _server method_ that will be entered, based on what the customer client signals they would
like to do.

Additionally, each party has some maintenance commands defined in their respective `manage.rs` file,
the customer defines its persistent escrow-watching daemon in
[`src/bin/customer/watch.rs`](src/bin/customer/watch.rs), and the merchant defines functionality for
connecting to an external approver service in
[`src/bin/merchant/approve.rs`](src/bin/merchant/approve.rs).

Without further ado, let's dive into the library:

## Configuration

The customer client/daemon are configured by a configuration file named `Customer.toml`, an example
of whose format can be found in the `dev/` directory. Likewise, the merchant server is configured by
a configuration file named `Merchant.toml`. Zeekoe will look for these files by default in the
operating system's preferred location for user-specific configuration files; to override this path,
you can specify the `--config` command line option to provide a path.

The format of the configuration files is defined in [`src/config.rs`](src/config.rs), and in the two
respective `config` modules, [`src/config/merchant.rs`](src/config/merchant.rs) and
[`src/config/customer.rs`](src/config/merchant.rs). The [Serde](https://serde.rs) package is used to
automatically support parsing the TOML configuration file format.

Many default values are shared across many parts of the application. These are defined in the
[`defaults`](src/defaults) module, and referred to in the derivation of the deserialization for
configurations. To change a default value or add a new one, look to that file.

## Command line interface

The command line interface for all the binaries is defined using the
[`structopt`](https://docs.rs/structopt/0.3.23/structopt/) crate, which generates a parser for
command line options based on annotations on Rust structs. The various structs which are parsed from
the command line options are defined in [`src/cli/customer.rs`](src/cli/customer.rs) and
[`src/cli/merchant.rs`](src/cli/merchant.rs).

## Database

Both the customer and the merchant persist local state using a database. The location of this
database is configured in the configuration file for each, and the path is interpreted relative to
the location of the configuration file. If the path specified in the file does not exist at the time
either agent is started, it will be created and initialized at the first time it is needed.
Currently, only SQLite databases stored locally are supported, but future support for Postgres or
other remote databases is on the road map.

Only a fixed set of database queries are required to implement the protocol, so they are defined
centrally, in the [`database`](src/database.rs) module and its submodules respectively for
[`customer`](src/database/customer.rs) and [`merchant`](src/database/merchant.rs).

A significant amount of logic goes into updates to the customer's persistent state; the types and
operations related to this are defined in
[`src/database/customer/state.rs`](src/database/customer/state.rs).

## Escrow

While the core zkChannels protocol is agnostic as to the escrow agent used to hold funds and confirm
validity of deposits, the current Zeekoe application supports only the Tezos cryptocurrency. All
escrow-specific code that directly interoperates with the Tezos blockchain is defined in
[`src/escrow/`](src/escrow), and in particular in [`src/escrow/tezos.rs`](src/escrow/tezos.rs).
These primitives for querying the blockchain and submitting operations are used throughout the
implementation of the protocol in the applications defined in the binaries.

## Protocol

The two-party protocol between the customer and the merchant is complex and without tool assistance
would be easy to make a mistake while implementing. In order to prevent such mistakes, the
[`dialectic`](https://docs.rs/dialectic) library is used to define a _session type_ corresponding to
the full zkChannels protocol performed between the merchant and customer. This protocol and all its
sub-protocols are defined as session types in the [`protocol.rs`](protocol.rs) module, alongside
some helper types, functions, and macros to make implementing them convenient.

## Transport

The transport layer atop which the above protocol is actually implemented is a TLS connection
implemented atop a TCP connection, which transmits messages by serializing them using the
[`bincode`](https://docs.rs/bincode) serialization format. It also supports an automatic
retry/resume mechanism which transparently reconnects dropped connections between the customer
client and the merchant server if they are disconnected due to a transient network error.

In deployment, the merchant server will need to be configured using a certificate that is part of
the WebPKI roots of trust. In development, any certificate can be used, and the customer client can
be instructed to trust an arbitrary single certificate using the `allow_explicit_certificate_trust`
cargo feature to build the customer client, and specifying the `trust_certificate` option in the
`Customer.toml` configuration file to point to the path of the certificate to be trusted.

The transport layer is broken up into several modules:

- [`client.rs`](src/transport/client.rs): the client-side transport layer, being a builder for clients
- [`server.rs`](src/transport/server.rs): the server-side transport layer, being a builder for servers
- [`channel.rs`](src/transport/channel.rs): the many layers of type definitions that describe the
  full transport layer as a backend to Dialectic
- [`handshake.rs`](src/transport/handshake.rs): the implementation of a session key and
  (re)connection handshake at the application layer, to allow for the retry/reconnect functionality
  to work
- [`io_stream.rs`](src/transport/io_stream.rs): a utility for testing that allows TLS to be optionally enabled
- [`pem.rs`](src/transport/pem.rs): helper functions for parsing PEM-encoded certificate files

## Amount

In the zkChannels protocol, amounts of money are unitless signed values. However, the exterior
interface of the application is in terms of currencies known to the user. The
[`src/amount.rs`](src/amount.rs) file defines an [`Amount`] type which is used across the
user-facing interface to parse currency amounts appropriately.
