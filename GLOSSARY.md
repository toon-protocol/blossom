# blossom

The TOON Protocol **blob app**: a Blossom server behind connector routes that keeps blobs people
have paid to store and serves them for free. Connector terms (**App**, **Route**, **Handler**,
**Price**, **Charge**) are defined in the connector's `CONTEXT.md` and are not repeated here.

## The app

**Blob app**:
This app: the one that keeps paid blobs and serves them by hash.
_Avoid_: Blob server, Blossom server, media server, store

**Bundle**:
One connector and one or more TOON apps behind it, stood up by one compose file. Every app runs
in a bundle of its own or in one shared with others, unchanged.
_Avoid_: Stack, deployment

**Shared edge**:
Running the blob app behind another bundle's connector, with its own connector off.
_Avoid_: Shared connector mode, co-hosting

**Store**:
The separate Arweave upload app (toon-protocol/store) behind `g.toon.store`; never this app.
_Avoid_: Using "store" for this app, its routes or its blobs

## Blobs

**Blob**:
The bytes of a file, kept outside any event and addressed by their sha256 hash.
_Avoid_: File, attachment, media, upload (as a noun)

**Holding**:
One owner's paid claim on a blob, with its own expiry. A blob's bytes are kept while any holding
on it is live.
_Avoid_: Lease, pin, copy, subscription

**Owner**:
The Nostr pubkey that signed the upload authorization behind a holding; the only party who may
end that holding.
_Avoid_: Uploader, author, payer

**Payer**:
The channel key the connector states paid for a request. Recorded, never an identity.
_Avoid_: Owner, customer

**Retention period**:
The fixed span of time one paid upload adds to its holding, set by the operator.
_Avoid_: TTL, lease

**Blob reference**:
An event's naming of a blob by its hash. It is data only: it neither requires the blob app to hold
the blob nor keeps it held.
_Avoid_: Dependency, attachment

## Routes and reads

**Upload route**:
The paid route by which a blob is written, priced by the blob's size.
_Avoid_: Store route, blob route, upload endpoint

**Mirror route**:
The paid route by which the blob app fetches a blob from another server itself, at a flat price
that buys up to a size limit.
_Avoid_: Copy route, import

**Media route**:
The paid route by which an image is optimized before it is kept, priced by the input's size.
_Avoid_: Transcode route, processing

**Blob read**:
A fetch of a blob by its hash from the public host; free, and never a route.
_Avoid_: Get route, download

**Preflight**:
A free check, on the public host, of whether a paid request would be accepted, run before paying.
_Avoid_: Quote, dry run

## Moderation

**Report**:
A signed complaint about a blob, kept for the operator. It never removes anything by itself.
_Avoid_: Flag, takedown

**Takedown**:
The operator's removal of a blob by hash, which ends every holding on it and refuses that hash
from then on.
_Avoid_: Ban, block, delete
