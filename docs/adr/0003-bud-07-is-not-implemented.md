# BUD-07 is not implemented

BUD-07 has the server answer `402` with its own Cashu or Lightning invoice and then check the
proof, which is payment validation inside the app, and all payment validation lives only in the
connector. An `X-TOON` method was considered and rejected: paid requests reach the blob app only
through the connector, already paid, so no `402` is ever needed on that path; stock Blossom
clients would ignore the header; and TOON clients already learn the price from the connector's
`GET /ilp`. A `PUT` sent directly to the public host is refused with `403`, saying that uploads go
through TOON at the upload route.
