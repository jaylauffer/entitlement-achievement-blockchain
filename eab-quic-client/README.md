# EAB QUIC Client

`eab-quic-client` is the small networking layer shared by Rust games using the
EAB game SDK. It opens direct QUIC connections for `eab-wire` secure messages
without depending on the EAB authority/server crate.

The client:

- requires an exact non-zero SHA-256 DER certificate fingerprint
- uses TLS 1.3 with ALPN `eab/2`
- disables early data and unidirectional streams
- verifies certificate validity, hostname, chain rules, and handshake
  signatures after matching the configured certificate pin
- limits each response to the `eab-wire` 64 KiB secure-frame maximum
- correlates every response with its 16-byte request id

It does not perform multicast discovery or decide which authority should be
trusted. A caller supplies an already selected direct endpoint and configured
pin. The game SDK's `QuicEabClaimTransport` adds session-bound claim submission
and exact-id status reconciliation.
