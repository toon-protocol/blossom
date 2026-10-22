# Blob storage on the relay: what fits behind the connector, and what it would take

> **Superseded in part.** This note was written in toon-protocol/relay and recommends building
> blobs into the relay. The design session that followed made the blob app its own app
> ([ADR 0001](../adr/0001-the-blob-app-is-its-own-app.md)) and left BUD-07 out
> ([ADR 0003](../adr/0003-bud-07-is-not-implemented.md)). The facts and citations still stand.

Research note. Accessed 2026-10-22. It recommends, but decides nothing; the
decision would be an issue here and, for the connector gaps, a record in the
connector's `docs/adr/`. Every substantive claim is cited inline; sources and
pins are at the foot. Three evidence labels are used throughout:

- **(verified)**: read in the owning spec, source file or first-party doc at the
  pinned commit.
- **(inference)**: my reasoning from verified facts. Everything in §6 and §7 is
  inference unless it carries a citation.
- **unverified**: I could not confirm it from a primary source.

**The question.** The relay should be able to store blobs (files) so that
events that depend on files can pay the relay for that storage through a paid
route on its connector. Which self-hosted storage solution does that?

**Citation shorthand.**
`CONN/<path>:L<n>` is `https://github.com/toon-protocol/connector/blob/b482f8017642f304bdddb45ebf9fe7ef76b7a85b/<path>#L<n>`.
`BUD/<nn>:L<n>` is `https://github.com/hzrd149/blossom/blob/b5bd2801d1763aa635fc8fea7a76597e0eb18990/buds/<nn>.md#L<n>`.
`NIPS/<file>:L<n>` is `https://github.com/nostr-protocol/nips/blob/d47a2ec8aacb9d01ac00e6294ae77582f483ff68/<file>#L<n>`.
`STORE/<path>:L<n>` is `https://github.com/toon-protocol/store/blob/98ead57c6b07ee74e5f34a3092db78bfd9e646f6/<path>#L<n>`.
A bare `path:L<n>` is toon-protocol/relay at `cceea16`, where this note was written.

---

## 1. Short answer

1. **Protocol: Blossom.** Its BUDs are the Nostr-native blob protocol, and NIP-96
   is now marked deprecated in favour of NIP-B7, which is Blossom
   (`NIPS/96.md:L1`, `NIPS/B7.md:L9`). Blobs are addressed by sha256, and that
   hash is how an event depends on a file: an `x` field in a NIP-92 `imeta` tag
   or a NIP-94 tag (§3).
2. **Storage: a content-addressed directory on the box's local disk, on a named
   volume.** Do not add an S3 server for one devnet box. MinIO's community
   repository is archived and says it is no longer maintained. Garage's own
   quick start warns that single-node Garage has no redundancy. A single-node
   S3 server would add a service without adding durability (§5).
3. **Server: build a small Blossom surface into the Rust relay.** Serve
   `PUT /upload`, `HEAD /upload` and `DELETE` on the write port, where only the
   connector can reach them, and `GET`/`HEAD /<sha256>` on the read port, free,
   like reads of events. The other option is to run a stock server (hzrd149's
   `blossom-server`, MIT, local or S3 backend) behind the connector as its own
   app. That works with no relay code and is the fastest way to prove the
   route. But no stock server can turn the connector's payment statement into a
   retention period, and turning payment into retention is the point of the
   request (§4, §7).
4. **Payment: no connector change is needed for pay-per-KiB on upload.** Routes
   already take `price = { base, per_kib }`, measured over the sealed payload,
   and the devnet's `g.toon.store` route is priced exactly that way
   (`deploy/connector.toml:L156-163`). The envelope already carries method,
   path, headers and body to the app, so a Blossom `PUT` with its
   `Authorization: Nostr …` header reaches the relay intact (§6).
5. **The hard gaps belong to the connector.** (a) A paid request body is capped
   at axum's default 2 MiB, with no config field to change it. (b) A
   duration-based price cannot be expressed; only length can. (c) Downloads
   cannot be priced per byte, because the schedule measures the request payload,
   not the response. (d) A refused upload (413, 409) is still charged. Section 6
   gives the workaround for each one that has a workaround, and §8 lists what
   the connector would need.

---

## 2. Constraints from this repo and the connector

**The relay speaks no ILP and validates no payment.** A request that reaches
`POST /write` is already paid. The relay records the connector's
`X-TOON-Payer` / `-Amount` / `-Chain` statement without re-checking it, and
"the connector's statement is the whole trust model"
(`crates/relay/src/write/payment.rs:L1-20`; connector ADR 0040). The store app
follows the same rule: it "trusts them and does not re-check them"
(`STORE/README.md:L49-51`). **(verified)**

**Only the write port is the paid door.** The write router serves `/write`,
`/write-ephemeral`, `/subscribe`, `/health`, `/metrics` and `/subscribers`
(`crates/relay/src/lib.rs:L267-274`). That port is `expose`d only on the
compose network, never published (`deploy/docker-compose.yml:L9-24`). Caddy
proxies the edge host to the connector and the read host to the relay's read
port, and nothing else (`deploy/Caddyfile`). The read port answers `426` to
anything that is not a WebSocket upgrade or a request for the NIP-11 document
(`crates/relay/src/read.rs:L5-8`). **(verified)**

**Prices are read, never held.** The relay reads its route, price and seal key
from the connector's `GET /ilp` (`crates/relay/src/edge.rs:L1-28`). It already
sees `pricePerKib` on a route (`crates/relay/src/route.rs:L77-78`). Today it
deliberately does not carry the slope into NIP-11
(`crates/relay/src/edge.rs:L136-142`), and the paid feed refuses a KiB-priced
subscribe route (`crates/relay/src/offer.rs:L34`). The paid feed's
`TOON_SUBSCRIBE_ILP_ADDRESS` (`docs/paid-feed.md`) is the precedent for "a
second paid route whose handler is the relay". **(verified)**

**What a route is.** A `[[routes]]` entry has a `prefix` (the ILP address
paid), a `handler_url` (the app endpoint) and a `price`
(`deploy/connector.toml:L97-112`). A route with a `handler_url` terminates
there. "The connector opens the sealed payload, makes exactly that HTTP request
of your app, and seals the app's complete response back"
(`CONN/docs/guide/put-your-app-behind-it.md:L3-5`). The envelope carries "a
method, a target, headers and a body going in; a status, headers and a body
coming back" (`CONN/crates/connector-domain/src/envelope.rs:L1-6`). The HTTP
client forwards method, headers (minus hop-by-hop ones) and body
(`CONN/crates/connector-runtime/src/app_client.rs:L189-241`). A sender's
target is resolved beneath the handler path. A relative target like
`orders/42` reaches `/write/orders/42`, and any other absolute path is refused
(`CONN/docs/adr/0025-…:L156-188`). **(verified)**

**Price is a schedule over payload length.** It is
`base + per_kib × ceil(payload_len / 1024)`, where `payload_len` is the sealed
wrap's length (`CONN/docs/adr/0065-a-price-is-a-schedule-over-payload-length.md:L9-13`).
The unit is the KiB, not the byte, because per-byte slopes round to zero
(`…0065…:L48-52`). The schedule is published in the x402 greeting, in
`GET /ilp` and in `GET /ilp/routes/price` (`…0065…:L93-96`). The price endpoint
accepts a `size` and returns the exact `charge` (`CONN/docs/protocol/client-edge-spec.md:L823-832`).
The app receives the actual charge as `X-TOON-Amount` (`…0065…:L131-135`). One
handler has one schedule. "An operator charges differently for different
_work_ by exposing a handler for each" (`…0065…:L117-123`). **(verified)**

**The ceiling and what a refusal costs.** "Today the only ceiling is the HTTP
body limit" (`…0065…:L139-143`). That limit is "axum's own `DefaultBodyLimit`
(2 MiB)", and "there is no config field for this"
(`CONN/docs/protocol/client-edge-spec.md:L101-103`). "You are paid for an
answer, not the answer the caller wanted. A `404` from your app … costs the
same as a `200`" (`CONN/docs/guide/put-your-app-behind-it.md:L23-25`). The
relay's paid feed records the same fact (`crates/relay/src/subscribe.rs:L17-18`).
An app that does not answer before the packet expires is refused `R00`
(`CONN/docs/adr/0064-…:L7-14`). **(verified)** The BTP carriage's frame or
message limit was not found in the connector's specs: **unverified**.

**Sealing has no small-payload cap.** The wrap is ChaCha20-Poly1305 under an
HKDF key, not NIP-44 (`CONN/crates/connector-signer/src/giftwrap.rs:L26-45`).
NIP-44's 65,535-byte plaintext limit therefore does not apply. Overhead is
fixed: a type byte, a 65-byte ephemeral key, a 12-byte nonce and a 32-byte
secret (`…giftwrap.rs:L38-40`), plus the AEAD tag. **(verified; that the tag is
16 bytes is inference from the cipher.)**

**The existing "store route" is Arweave, not self-storage.** `g.toon.store`
(and `g.toon.relay.store`) are routes to `store-dvm:3300/store`, priced
`{ base = 1000, per_kib = 10 }` (`deploy/connector.toml:L154-163`). The store
answers NIP-90 `kind:5094` jobs by uploading the bytes to Arweave via Turbo and
returning a transaction id (`STORE/README.md:L7-15`, `L42-47`). Storage there is
permanent and bought in $ARIO per upload (`STORE/README.md:L161-168`). It is
the in-house precedent for a per-KiB storage route. It is not a store on this
box. **(verified)**

---

## 3. Protocol options

| Spec | Status | What it gives | Verdict |
| --- | --- | --- | --- |
| **Blossom BUD-01** | `draft` `mandatory` (`BUD/01:L5`) | `GET`/`HEAD /<sha256>[.ext]`, Range, CORS `*`. Endpoints MUST be at the root of the domain (`BUD/01:L24`). Optional `Sunset` header for a known retention end (`BUD/01:L101-111`). | Serve on the read host, free. |
| **BUD-02** | draft, optional | `PUT /upload` with a binary body. The server MUST hash the exact bytes; optional `X-SHA-256`; `201`/`200` with a Blob Descriptor; `402` and `413` defined (`BUD/02:L39-67`). | The paid write. |
| **BUD-04** | draft, optional | `PUT /mirror {url}`, so the server fetches the blob itself (`BUD/04:L11-20`). | Useful later. The fetched size is unknown when the price is charged, so price it flat or skip it. |
| **BUD-06** | draft, optional | `HEAD /upload` preflight on `X-SHA-256`, `X-Content-Length` and `X-Content-Type`, answering `402`/`413`/`415` (`BUD/06:L7-19`). | Needed: a refused paid upload still costs money, so clients must be able to check first, for free. |
| **BUD-07** | draft, optional | `402` plus an `X-{method}` header. It defines `X-Cashu` (NUT-24) and `X-Lightning` (BOLT-11). New methods need their own public spec (`BUD/07:L11`, `L27-30`, `L99-103`). | Does not fit as written. See §6.4. |
| **BUD-08** | draft, optional | A `nip94` tag array in the descriptor (`BUD/08:L13`). | Cheap; lets clients paste tags straight into events. |
| **BUD-11** | draft, optional | Auth tokens are kind `24242` events with an `expiration`, a `t` verb and `x` hash scoping. `PUT /upload` requires a matching `x` (`BUD/11:L11-27`, `L77`). Sent as `Authorization: Nostr <base64url>` (`BUD/11:L50`). | Keep it for owner identity. It is not authorization: the payment authorizes. |
| **BUD-12** | draft, optional | `DELETE /<sha256>`; `/list/<pubkey>` "optional and unrecommended" (`BUD/12:L15`, `L41`). | Delete, yes. List, no. |
| BUD-03 | draft, optional | kind `10063` user server list (`BUD/03:L7-11`). | Client-side. The relay's blob host goes in users' lists. |
| BUD-10 | draft, optional | `blossom:<sha256>.<ext>?xs=…` URIs (`BUD/10:L11-30`). | Client-side. |
| **NIP-B7** | draft, optional | Clients SHOULD use kind 10063 lists and treat a trailing 64-hex string in a URL as a sha256 to find elsewhere, verifying the hash on download (`NIPS/B7.md:L9-32`). | The way a dependent event survives its server going away. |
| NIP-96 | `unrecommended`, "deprecated in favor of NIP-B7" (`NIPS/96.md:L1-9`) | REST upload API. | Do not build. |
| NIP-94 | draft, optional | kind `1063` file metadata event: `url`, `m`, `x` (sha256), `ox`, `size`, … (`NIPS/94.md:L11-27`). | A standalone "this file" event, when a file has no parent event. |
| NIP-92 | (no status line) | `imeta` tag on any event: a `url` plus NIP-94 fields, including `x` (`NIPS/92.md:L7-13`, `L22-31`). | How a note or other event depends on a file. |

**How an event depends on a file (verified, from the specs above).** The event
carries the blob's URL and, in `imeta` (NIP-92) or a NIP-94 `x` tag, its
sha256. Any server holding the same bytes can serve it, because the hash is the
address. NIP-B7 makes clients look the hash up on the author's other servers
when the URL dies. Given that, the relay could keep the reference graph as a
side index: from the `x` values in stored events it can tell which blobs are
referenced and by whom. **(inference)**

---

## 4. Self-hostable Blossom server implementations

Pinned commits are in [Sources](#sources). Language and licence come from the
repository. "BUD-07" means whether the server can emit `402` plus payment
headers.

| Server | Lang | Licence | Storage / metadata | BUDs claimed | Payment hooks | Activity | Fit behind the connector |
| --- | --- | --- | --- | --- | --- | --- | --- |
| [hzrd149/blossom-server](https://github.com/hzrd149/blossom-server) (the spec author's) | TypeScript, Deno 2 + Hono | MIT (`LICENSE.txt`) | local dir or S3 (optional redirect to a public S3 URL); SQLite/libSQL | 01, 02, 04, 05, 06, 08, 09, 11 (README L8-15) | None listed in the README (**unverified** in code) | pushed 2026-10-07 | Good as a payment-oblivious app. Retention rules key on MIME and pubkey, and expiry is "how long a blob may go unaccessed" (`config.example.yml` L83-102). Nothing maps payment to retention. Default `maxSize` is 2 GB (L127-129). |
| [v0l/route96](https://github.com/v0l/route96) | Rust (axum 0.8, `nostr` 0.44) | **None**: no LICENSE file and no `license` in `Cargo.toml` (verified at pin) | local `storage_dir`; **MySQL/MariaDB required** | 01, 02, 04, 05, 06, 08, 09 + "BUD-13" from an unmerged `simpler-put` branch (README L11-19) | Built-in LND Lightning, quotas, `402` with `X-Lightning`/`X-Cashu` (`src/routes/blossom.rs` L244-262) | pushed 2026-10-21 | Its own billing duplicates what the connector does. With no licence, it cannot be redistributed or modified (inference, not legal advice). Reject. |
| [flox1an/almond](https://github.com/flox1an/almond) | Rust | README says MIT; **no LICENSE file at root** | local FS or S3, **no database** | (compliance doc in repo) | Cashu BUD-07, reactive and pre-emptive (`CONTEXT.md` L156-175) | v0.5.2 released 2026-10-22; 5 stars | The closest feature fit: `ALMOND_UPLOAD_MAX_AGE` is age-based retention, emitted as `Sunset` (`.env.example`). But it is very young, and `UPLOAD_ACCESS=public` by default means anyone may upload if `PUT` is exposed. |
| [PrimalHQ/primal-blossom-server](https://github.com/PrimalHQ/primal-blossom-server) | Rust (nightly in README) | MIT | local + CDN; **PostgreSQL required** | (README lists features, not BUDs) | none found | pushed 2026-10-14 | Heavy for one box. |
| [sebdeveloper6952/blossom-server](https://github.com/sebdeveloper6952/blossom-server) | Go | Unlicense | (sqlc DB) | 01, 02, 04, 06, 08 | none listed | pushed 2026-10-10 | Possible; small. |
| [0xtrr/cherry-server](https://github.com/0xtrr/cherry-server) | Rust | MIT | disk + SQLite | 01, 02, 04, 06 | none listed | pushed 2026-06-15 | Possible; stale-ish. |
| [barrydeen/haven](https://github.com/barrydeen/haven) | Go | MIT | relay + Blossom; files at `BLOSSOM_PATH` | n/a | n/a | pushed 2026-10-15 | It is a different relay. Not applicable. |
| [MonumentalSystems/blossom-rs](https://github.com/MonumentalSystems/blossom-rs) (crate `blossom-rs` 0.6.1, MIT) | Rust library + server | MIT | trait `BlobBackend` (memory / FS / S3); SQLite / Postgres | Claims "BUD-06" at `GET /upload-requirements` and "BUD-19/20" (README L118-128) | `AccessControl`, `WebhookNotifier` traits | crate updated 2026-08-12 | Embeddable, but it diverges from the spec: BUD-06 is `HEAD /upload` (`BUD/06:L7`), and BUDs stop at 12 in the spec repo. |
| `nostr-blossom` 0.45.1 (rust-nostr) | Rust | MIT | n/a | 01-12 | client only | crate 2026-09-11 | README: "**Server:** Not implemented", "ALPHA state". It pulls `reqwest`. It gives types, not a server. |

**Can a stock server sit behind the connector the way the relay does?** Yes,
mechanically **(verified)**. The connector delivers any method, target and
headers beneath a handler path (§2). Take `handler_url =
"http://blossom:3000/"` and an envelope `PUT` with target `upload`: it arrives
as `PUT /upload` with `Authorization`, `Content-Type` and `X-SHA-256` intact.
The connector strips any caller-written `X-TOON-*` and adds its own
(`CONN/docs/guide/put-your-app-behind-it.md:L13-19`).

Three things must hold **(inference)**:

- Caddy must expose only `GET`, `HEAD` and `OPTIONS` of that server publicly.
  `PUT` and `DELETE` must be reachable only from the connector. Otherwise the
  free door the privacy invariant exists to prevent is back.
- The server keeps its own policy (auth, size, retention) and cannot see what
  was paid.
- A free route sharing that origin would let one path be reached at another's
  price. ADR 0025's confinement holds only for distinct handler paths, so each
  priced verb needs its own path.

**Rust crates for building it in.** There is no maintained server crate that
follows the spec closely enough to embed. `nostr-blossom` is a client,
`blossom-rs` diverges, and `route96` has no licence. The relay already depends
on `nostr` (`=0.45.4`), `sha2`, `axum` 0.8 and `rusqlite`
(`Cargo.toml:L30-50`). That covers sha256 addressing, kind 24242 signature
checks, the HTTP surface and a metadata table. **(verified for the
dependencies; "enough" is inference)**

---

## 5. Storage backends

| Backend | Licence | Single devnet box | Replication | S3 API | Verdict |
| --- | --- | --- | --- | --- | --- |
| **Local filesystem, content-addressed** (`<root>/ab/cd/<sha256>`) on a named Docker volume | n/a | Nothing to run. Atomic write with temp file + `fsync` + `rename` (inference). | None. Back up the volume. | n/a | **Use this.** |
| MinIO community | AGPL-3.0 | **Repo archived**, README: "THIS REPOSITORY IS NO LONGER MAINTAINED", alternatives are AIStor Free/Enterprise. "Distributed as source code only"; last release `RELEASE.2025-10-15T17-29-55Z`; last push 2026-04-24 ([minio/minio](https://github.com/minio/minio) README at pin). | (erasure sets) | yes | **Reject.** |
| [Garage](https://garagehq.deuxfleurs.fr) (Deuxfleurs) | AGPL-3.0 | `garage server --single-node` (v2.3.0+) sets `replication_factor = 1`. The quick start says this "should not be used in production, as it provides no redundancy for your data!" ([quick start](https://garagehq.deuxfleurs.fr/documentation/quick-start/)) | Geo-distributed, its reason to exist (README) | Subset: no versioning, object lock, bucket policies or ACLs. Lifecycle `Expiration` is partial; multipart and presigned URLs work ([S3 compatibility](https://garagehq.deuxfleurs.fr/documentation/reference-manual/s3-compatibility/)). | The best choice **when there are 3+ boxes**. Not before. |
| [SeaweedFS](https://github.com/seaweedfs/seaweedfs) | Apache-2.0 (an Enterprise Edition exists) | One `weed` binary; `weed mini` "is fine for single-node production" (README L89) | `replicationPlacement`, active-active (README L41-44, L127-128) | "The most complete S3 API" (its own claim, README L41) | The best permissive alternative if S3 is ever needed on one box. Release 4.48, 2026-09-28. |
| [RustFS](https://github.com/rustfs/rustfs) | Apache-2.0 | Docker compose; 1.0.3 released 2026-10-17 | distributed | MinIO-compatible (its own claim) | 1.0 is weeks old. Production maturity **unverified**. |
| Ceph RGW | LGPL-2.1/3.0, mostly ([ceph/ceph](https://github.com/ceph/ceph) README L12-14) | Needs a whole Ceph cluster under it (inference) | yes | yes | Too heavy for a devnet box. |

On licences: AGPL obligations attach to modifying and offering the AGPL
program itself. Running unmodified Garage or MinIO as a separate service would
not reach the relay's MIT code. **(inference, not legal advice)**

On the box: nothing in this repo states the devnet relay box's disk size or
how its volumes are backed up, so both are **unverified**. Measure them before
setting a storage cap and enabling uploads.

---

## 6. Payment and route mapping

All of §6 is inference built on the verified facts in §2, except where cited.

### 6.1 What works today, with no connector change

```toml
# deploy/connector.toml — illustrative, not committed
[[routes]]
prefix      = "g.toon.relay.blob"
handler_url = "http://relay:3100/blob/"      # trailing slash load-bearing
price       = { base = 1000, per_kib = 10 }  # same schedule as g.toon.store
```

- A client builds a Blossom upload as an envelope: `PUT`, target `upload`,
  headers `Content-Type`, `X-SHA-256` and `Authorization: Nostr <kind 24242>`,
  and the body as raw bytes. It seals the envelope and pays
  `g.toon.relay.blob`. The charge is fixed by the sealed length. The client
  computes it beforehand from `GET /ilp` or from
  `GET /ilp/routes/price?destination=…&size=…`.
- The relay receives `PUT /blob/upload` on the write port. It hashes the body,
  checks `X-SHA-256` and the 24242 `x` tag, stores the blob, and records
  `X-TOON-Payer`, `X-TOON-Amount`, `X-TOON-Chain`, the owner pubkey (from
  24242), the size and the expiry. It answers `201` with a Blob Descriptor
  (BUD-02) whose `url` is on the read host.
- At the devnet store schedule, 1 MiB costs 1000 + 10 × 1024 = 11,240 base
  units, about 0.011 USDC.
- Reads are `GET`/`HEAD /<sha256>` on the read host: free, never through the
  connector, as reads of events are. Stock Blossom clients and NIP-B7 fallback
  work unchanged, because BUD-01 only needs the domain root.

### 6.2 Size: the 2 MiB wall

The paid door is `POST /ilp`. Its body limit is 2 MiB, with no config field.
That covers the sealed payload plus ILP framing, so a blob of about 2 MiB less
a few hundred bytes is the largest that fits. Images fit. Video and archives do
not. Options, from cheapest:

1. Advertise `413` above about 1.9 MiB in `HEAD /upload` (BUD-06) and accept
   the cap on devnet.
2. Use BTP carriage, if its frame limit is larger (**unverified**; see §2).
3. Chunk: upload N paid parts, each its own blob, plus a manifest. That is not
   standard Blossom. Stock clients cannot reassemble it.
4. **A connector change:** a configurable per-route body limit (ADR 0065 left
   `max_bytes` to its own record, `…0065…:L139-143`), or a raised default.

### 6.3 Duration: price buys a fixed retention, or tiers by handler

The connector cannot price time. ADR 0065 prices length only and explicitly
declines app-quoted prices (`…0065…:L145-152`). So:

- **One policy:** one paid upload buys a fixed retention T, an operator
  setting. The descriptor and every `GET` carry `Sunset` (BUD-01). Paying
  again for the same sha256 returns `200` (already stored, per BUD-02) and
  extends the expiry by T. Duration becomes "pay again", priced by size.
- **Tiers:** ADR 0020/0065 already allow one handler per price. For example,
  `g.toon.relay.blob` → `/blob/30d/` at `{1000, 10}` and
  `g.toon.relay.blob.year` → `/blob/365d/` at `{5000, 100}`. The relay reads
  the tier from the path it was called on, which the operator sets, never the
  sender. No connector change is needed.
- Not possible without the connector: "N bytes × D days" with D chosen by the
  client.

### 6.4 Why BUD-07 does not fit, and how it relates to x402

BUD-07 has the app itself answer `402` with an `X-{method}` invoice and then
check the proof (`BUD/07:L11`, `L60-80`). That is payment validation inside the
app, which this project forbids. In TOON the `402` comes from the connector, as
the x402 greeting on an unpaid `POST /ilp`
(`CONN/docs/protocol/client-edge-spec.md:L97-100`), never from the relay. The
existing x402 note already found that a stock x402 client cannot buy from a
TOON app, because the paid side is a sealed packet
(`docs/x402-bazaar-on-relay.md:L49-57`).

The consequence: **uploads need a TOON-aware client.** Stock Blossom clients
can read but cannot pay to write. A future `X-TOON` BUD-07 method, a public
spec that points at the connector's greeting, would let Blossom clients
discover the price. The paying would still happen at the connector.

### 6.5 Refusals, deletes and egress

- **A refusal still costs its price**, because a `413`, `409` or `415` rides
  home on a FULFILL. So: `HEAD /upload` must be on the free read port, with the
  same checks as the paid path. The `Authorization` sha256 must match the body
  before storing, or the upload is refused (charged; the client's own fault).
  Clients should preflight first.
- **Delete (BUD-12)** at the blob route's price is acceptable. A free delete
  would be its own `price = 0` route to `/blob-delete/`. Like
  `/write-ephemeral`, it must then fully verify the 24242 signature, because no
  payment gates it (`deploy/connector.toml:L114-133` is the precedent).
- **Egress is unpriced.** The schedule measures the request payload, so a
  download cannot be priced by size even through the connector. Reads stay free
  and are bounded by rate limiting (the relay has a limiter for its free lane,
  `crates/relay/src/write/limiter.rs:L1`) and by Caddy. Paid egress would need
  a connector change: a price on the response length, which no ADR proposes.
- **Payer is not owner.** `X-TOON-Payer` is a channel key. The blob's owner is
  the 24242 pubkey. Record both. Delete is by owner, and the payer is for the
  operator's records (ADR 0036/0040 intent).

---

## 7. Recommended architecture

```
                         ╔══════════════════════════════════════════════════╗
 uploader ── POST /ilp ─▶║ Caddy ─▶ connector ─┬─ g.toon.relay       ─▶ :3100 /write        (events)
  (TOON client,          ║  :443      :3000    └─ g.toon.relay.blob  ─▶ :3100 /blob/upload  (PUT, paid per KiB)
   sealed PUT /upload)   ║                                              │                       ║
                         ║                                              ▼                       ║
 reader ── wss:// ──────▶║ Caddy ───────────────────────────────▶ relay :7100  REQ/EVENT (free)  ║
 any Blossom client ────▶║   READ_HOST  GET|HEAD /<sha256>[.ext]   │     GET/HEAD/<sha256> (free)║
   GET /<sha256>         ║              HEAD /upload (preflight)   │     HEAD /upload (free)     ║
                         ║                                         ├── events.db  (+ blobs table:║
                         ║                                         │     sha256, size, type,     ║
                         ║                                         │     owner, payer, amount,   ║
                         ║                                         │     chain, expires_at)      ║
                         ║                                         └── blobs/ab/cd/<sha256>      ║
                         ╚══════════════════════════════════════════════════════════════════════╝
```

**Opinionated choice:** a relay-owned Blossom subset, on a local
content-addressed directory. It would cover BUD-01, 02, 06, 08, 11 and the
delete half of 12. It needs no S3 server and no second app. Reasons:

- **The payment-to-retention coupling is the feature.** It needs code that
  reads the connector's statement, and only the relay's write handler may build
  that statement (`crates/relay/src/write/payment.rs:L15-21`). A stock server
  ignores it.
- **Events and blobs share a lifecycle.** The relay already enforces
  NIP-09/NIP-40 (`docs/retention.md`), and it can see `imeta` `x` references.
  That allows "keep while referenced and paid", and lets an operator blocklist
  apply to blobs.
- **It is the same shape as the paid feed.** The address comes from config
  (`TOON_BLOB_ILP_ADDRESS`, confirmed against `GET /ilp` the way
  `TerminatedRoute` confirms the write route). The price and slope are read,
  never held. NIP-11's `toon` object can name the blob route and its slope, but
  only once `edge.rs` stops dropping `pricePerKib` for this route.
- **The conformance suite stays the judge.** A Blossom surface is HTTP on the
  image's ports, which is what `packages/conformance` already tests.

**Fastest proof first, if wanted:** run `hzrd149/blossom-server` (MIT, local
backend) as `blossom:3000` behind the route above. Expose only
`GET/HEAD/OPTIONS` through Caddy, and set one retention rule. No relay code
changes. It proves the client flow, the sizes and the prices. Then replace it
with the relay-owned surface when retention must follow payment.

**When to add S3:** when there is a second box, or the disk is the bottleneck.
Then use Garage (3 nodes, geo-replicated, AGPL) or SeaweedFS (Apache-2.0, one
binary). Not MinIO.

---

## 8. Open questions and gaps

| # | Gap | Owner | Needed for |
| --- | --- | --- | --- |
| 1 | `POST /ilp` body limit fixed at 2 MiB, with no config (`client-edge-spec.md:L101-103`) | connector | Blobs > ~2 MiB over HTTP carriage |
| 2 | BTP carriage message/frame limit | connector (**unverified**) | Knowing whether BTP lifts #1 |
| 3 | Per-route `max_bytes` refusal, deferred by ADR 0065 | connector | Refusing oversize before the app is charged for |
| 4 | No price on time, and app-quoted pricing declined (ADR 0065) | connector | Client-chosen retention |
| 5 | No price on response length | connector | Paid egress |
| 6 | Refused work is charged (FULFILL on `4xx`) | by design | Mitigated by a free `HEAD /upload` |
| 7 | NIP-11 drops `pricePerKib` (`edge.rs:L136-142`) | relay | Advertising the blob route's real price |
| 8 | No Blossom method for TOON payment in BUD-07 | spec (a new public doc) | Stock Blossom clients discovering how to pay |
| 9 | Should the relay refuse an event whose `imeta` `x` it does not hold? | relay policy | Strong "depends on" semantics. Default should be no: NIP-B7 lets files live elsewhere. |
| 10 | Disk size and backups of the devnet box | operator (**unverified**) | Setting `STORAGE_MAX` and T |

---

## Sources

toon-protocol/relay at `cceea16`: `CLAUDE.md`, `README.md`,
`crates/relay/src/{lib.rs,edge.rs,route.rs,read.rs,offer.rs,subscribe.rs,write.rs,write/payment.rs,write/limiter.rs}`,
`Cargo.toml`, `deploy/{connector.toml,docker-compose.yml,Caddyfile}`,
`docs/{x402-bazaar-on-relay.md,paid-feed.md,retention.md}`.

toon-protocol/connector @ `b482f8017642f304bdddb45ebf9fe7ef76b7a85b`:
`docs/adr/0020-a-price-is-flat-and-attaches-to-a-handler.md`,
`docs/adr/0025-an-envelope-target-is-confined-beneath-the-handler-path.md`,
`docs/adr/0040-a-verified-payment-is-stated-to-the-app.md`,
`docs/adr/0064-a-deadline-bounds-the-wait-for-an-app-not-the-answer.md`,
`docs/adr/0065-a-price-is-a-schedule-over-payload-length.md`,
`docs/adr/0067-a-route-declares-its-request-shape-and-the-connector-never-reads-it.md`,
`docs/guide/put-your-app-behind-it.md`, `docs/protocol/client-edge-spec.md`,
`crates/connector-domain/src/envelope.rs`,
`crates/connector-runtime/src/app_client.rs`,
`crates/connector-signer/src/giftwrap.rs`.

toon-protocol/store @ `98ead57c6b07ee74e5f34a3092db78bfd9e646f6`: `README.md`,
`src/store-backend.ts`, `src/entrypoint-store.ts`.

Blossom spec: hzrd149/blossom @ `b5bd2801d1763aa635fc8fea7a76597e0eb18990`,
`README.md`, `buds/00.md`–`buds/12.md` (Unlicense, "Public domain").

NIPs: nostr-protocol/nips @ `d47a2ec8aacb9d01ac00e6294ae77582f483ff68`:
`B7.md`, `96.md`, `94.md`, `92.md`.

Servers (README / config / source at commit):
hzrd149/blossom-server `a492dc61c4a581bbd0992546b2aec6f9aa543f75` (`README.md`, `config.example.yml`);
v0l/route96 `f1eb2b5dee64273e676a5f3bb3e4d30893a2f5f5` (`README.md`, `Cargo.toml`, `src/routes/blossom.rs`);
flox1an/almond `c049716ea723306687ed7c629508fe9049202232` (`README.md`, `.env.example`, `CONTEXT.md`);
PrimalHQ/primal-blossom-server `7e595c9c5b6e393de80edb9fe32db20bc5ffc55a`;
sebdeveloper6952/blossom-server `ba1444c31d517de9fcb512f7fff92bfed421aaa7`;
0xtrr/cherry-server `037fb2adf41ab6233c0d2b567624959f28c1995b`;
barrydeen/haven `60b58ea762d40e345bfeb380c6678b50c977f381`;
MonumentalSystems/blossom-rs `fc4a6e72aa8c2c1e728c0ae94217a86bc47feff5`;
nostrdevkit/nostr `20096765b95f4840dd3df66cad49db35e9634462` (`nostr-blossom/README.md`, `Cargo.toml`);
crates.io API for `nostr-blossom` 0.45.1 (MIT) and `blossom-rs` 0.6.1 (MIT).

Storage backends (README at commit, plus first-party docs):
minio/minio `7aac2a2c5b7c882e68c1ce017d8256be2feea27f` (archived; GitHub releases API);
deuxfleurs-org/garage `54c928d521645ad032902af68e04d12bc6000d5e` (mirror of git.deuxfleurs.fr),
https://garagehq.deuxfleurs.fr/documentation/quick-start/ and
https://garagehq.deuxfleurs.fr/documentation/reference-manual/s3-compatibility/;
seaweedfs/seaweedfs `137d29633b2b5d62da7fa63b0da7b43caff08822`;
rustfs/rustfs `6e3484916fd688800b8e5fb5dda7a55dd8175473`;
ceph/ceph `77807d71e9a64c344ae67e06615eafd99863fc63`.
