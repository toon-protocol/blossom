# The blob app is its own app, not part of the relay

Blobs could have been a Blossom surface built into the Rust relay, which already reads the
connector's payment statement and stores events. We made the blob app its own app, with its own
repo, image and routes, beside the relay behind the same connector, as the store and gas station
are. Events and blobs share no lifecycle (a blob reference is data only, so the relay never keeps
a blob because of an event), and media optimization (BUD-05) brings image libraries and CPU-heavy
work that should not ship in the image the live relay box follows on every merge.

## Consequences

- Payment still decides retention: the blob app reads `X-TOON-Payer` / `-Amount` / `-Chain` the
  way the relay does. That was only ever impossible for a stock Blossom server.
- The blob app's public host is its own (Blossom endpoints must sit at the domain root), and the
  relay's NIP-11 document does not advertise it.
- The relay's code for reading the connector's `GET /ilp` and confirming a route is copied here,
  not shared, until a third app needs it.
