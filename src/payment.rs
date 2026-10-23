use axum::http::HeaderMap;

use crate::error::ApiError;

/// The connector's statement of what paid for a request (`X-TOON-Payer` / `-Amount` / `-Chain`).
///
/// It is recorded and trusted, never checked. The payer is not an identity: ownership comes from
/// the upload authorization's signer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payment {
    payer: String,
    amount: u128,
    chain: String,
}

impl Payment {
    /// The channel key that paid: never an identity.
    #[must_use]
    pub fn payer(&self) -> &str {
        &self.payer
    }

    /// The amount the connector states was paid.
    #[must_use]
    pub fn amount(&self) -> u128 {
        self.amount
    }

    /// The chain the payment settles on (for example `evm:31337`).
    #[must_use]
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// # Errors
    /// [`ApiError::PaymentStatement`] when a header is missing, empty or (for the amount) not an
    /// unsigned integer.
    pub fn from_headers(headers: &HeaderMap) -> Result<Self, ApiError> {
        let text = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or(ApiError::PaymentStatement)
        };
        Ok(Self {
            payer: text("x-toon-payer")?.to_owned(),
            amount: text("x-toon-amount")?
                .parse()
                .map_err(|_| ApiError::PaymentStatement)?,
            chain: text("x-toon-chain")?.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(k, v)| (k.parse().unwrap(), v.parse().unwrap()))
            .collect()
    }

    #[test]
    fn reads_a_full_statement() {
        let h = headers(&[
            ("x-toon-payer", "abc"),
            ("x-toon-amount", "1010"),
            ("x-toon-chain", "evm:31337"),
        ]);
        let payment = Payment::from_headers(&h).unwrap();
        assert_eq!(payment.amount, 1010);
        assert_eq!(payment.chain, "evm:31337");
    }

    #[test]
    fn refuses_a_partial_or_malformed_statement() {
        assert!(Payment::from_headers(&HeaderMap::new()).is_err());
        let h = headers(&[("x-toon-payer", "abc"), ("x-toon-amount", "1010")]);
        assert!(Payment::from_headers(&h).is_err());
        let h = headers(&[
            ("x-toon-payer", "abc"),
            ("x-toon-amount", "-5"),
            ("x-toon-chain", "c"),
        ]);
        assert!(Payment::from_headers(&h).is_err());
    }
}
