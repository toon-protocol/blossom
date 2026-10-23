//! A `Payment` is what the connector stated on the delivery, so it cannot be made up in a handler.

use blossom::payment::Payment;

fn main() {
    let _ = Payment {
        payer: "0xchannelkey".to_owned(),
        amount: 1010,
        chain: "evm:31337".to_owned(),
    };
}
