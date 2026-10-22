# A payment buys a fixed retention period

The connector prices a request by its size only (connector ADR 0065), so storage time cannot be
priced. One paid upload buys a holding for a fixed retention period set by the operator, and
uploading the same blob again extends that owner's holding to max(now, current expiry) plus the
period. Retention by reference ("kept while an event names it") was rejected because anyone could
keep a blob alive by publishing a cheap event, and "kept until deleted" because it leaves the
operator's cost unbounded.

## Consequences

- Each upload is a holding of its own. Identical bytes uploaded by two owners are kept once, and
  one owner's delete does not end the other's holding.
- The expiry is advertised with Blossom's `Sunset` header.
- Longer periods can be sold later as separate routes at their own prices, without changing the
  model.
