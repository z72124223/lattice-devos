# LATTICE postgres 0.19.14 patch

This directory is an audited copy of the crates.io `postgres` 0.19.14 package
whose published checksum is
`33ad20e0aa0b24f5a394eab4f78c781d248982b22b25cecc7e3aa46a681605bd`.

LATTICE adds one synchronous configuration method,
`Config::connect_with_startup_timeout`. It wraps the complete asynchronous
startup/authentication future in `tokio::time::timeout`, returns `Ok(None)` on
deadline expiry, and drops the in-flight future and socket before returning.
All existing upstream APIs and dependency versions remain unchanged.

The patch exists because upstream `Config::connect_timeout` covers socket
connection attempts only. LATTICE needs an absolute fail-closed deadline for
authentication and PostgreSQL startup before any managed-provider effect.
