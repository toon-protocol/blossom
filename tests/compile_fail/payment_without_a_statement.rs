//! A `Payment` is read from the connector's statement headers, so it cannot be built field by field.

use blossom::payment::Payment;

fn main() {
    let _ = Payment {
        payer: "0xchannelkey".to_owned(),
        amount: 1010,
        chain: "evm:31337".to_owned(),
    };
}
