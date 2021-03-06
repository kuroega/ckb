# CKB Core Development

## Running Test

Install dependencies

```
rustup component add rustfmt
rustup component add clippy
```

Run tests

```
make ci
```

Run acceptance integration tests


```
cargo build
cd test && cargo run ../target/debug/ckb
```

## Chain Spec

The subcommand `ckb init` has an option `--export-specs` to export spec files
as well, which allows editing the chain spec for development.

The chain spec can switch between different PoW engines. Wiki has the [instructions](https://github.com/nervosnetwork/ckb/wiki/PoW-Engines) about how to configure it.-
