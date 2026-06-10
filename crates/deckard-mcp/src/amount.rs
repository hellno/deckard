//! Exact decimal-ETH ⇄ wei conversion for the tool schemas.
//!
//! `amount_eth` rides the wire as a **decimal string** ("0.02"), never a JSON number: an
//! f64 cannot represent wei exactly (a funds bug waiting to happen), and wei-as-JSON-number
//! overflows many JSON readers. The parse is exact and total — junk is rejected with an
//! actionable message, never silently coerced to a wrong magnitude. (Mirrors the app's
//! amount-field parser in `deckard-app/src/signer.rs`.)

use alloy_primitives::U256;

/// Parse a decimal ETH amount (`"0.05"`, `"1"`, `"1.234"`) into wei. Rejects empties,
/// signs, non-digits, a second dot, and >18 fractional places.
pub fn parse_eth_to_wei(input: &str) -> Result<U256, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("amount_eth is empty — pass a decimal ETH string like \"0.02\"".into());
    }
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err("amount_eth must be a decimal ETH string like \"0.02\"".into());
    }
    let all_digits = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(int_part) || !all_digits(frac_part) {
        return Err(
            "amount_eth must contain only digits and at most one dot (decimal ETH, not wei, \
             no sign, no separators) — like \"0.02\""
                .into(),
        );
    }
    if frac_part.len() > 18 {
        return Err("amount_eth has too many decimal places (max 18 for ETH)".into());
    }
    // Concatenate the integer part with the fractional part right-padded to 18 digits → wei.
    let mut digits = String::with_capacity(int_part.len() + 18);
    digits.push_str(if int_part.is_empty() { "0" } else { int_part });
    digits.push_str(frac_part);
    for _ in frac_part.len()..18 {
        digits.push('0');
    }
    U256::from_str_radix(&digits, 10).map_err(|_| "amount_eth is too large".into())
}

/// Render wei as a trimmed decimal-ETH string (`"0.02"`, `"1"`, `"1.234"`), exact.
pub fn format_wei_as_eth(wei: U256) -> String {
    let s = wei.to_string();
    let (int_part, frac_part) = if s.len() > 18 {
        let split = s.len() - 18;
        (s[..split].to_string(), s[split..].to_string())
    } else {
        ("0".to_string(), format!("{s:0>18}"))
    };
    let frac = frac_part.trim_end_matches('0');
    if frac.is_empty() {
        int_part
    } else {
        format!("{int_part}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_decimals() {
        assert_eq!(parse_eth_to_wei("1").unwrap(), U256::from(10u128.pow(18)));
        assert_eq!(
            parse_eth_to_wei("0.05").unwrap(),
            U256::from(50_000_000_000_000_000u128)
        );
        assert_eq!(
            parse_eth_to_wei(" 0.02 ").unwrap(),
            U256::from(20_000_000_000_000_000u128)
        );
        assert_eq!(
            parse_eth_to_wei("0.000000000000000001").unwrap(),
            U256::from(1u64)
        );
        assert_eq!(parse_eth_to_wei("0").unwrap(), U256::ZERO);
    }

    #[test]
    fn rejects_junk_loudly() {
        for bad in [
            "",
            " ",
            ".",
            "abc",
            "-1",
            "+1",
            "1.2.3",
            "1,5",
            "0.1234567890123456789",
            "1e18",
            "0x10",
        ] {
            assert!(parse_eth_to_wei(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn formats_round_trip() {
        for s in ["1", "0.05", "0.02", "1.234", "0.000000000000000001", "0"] {
            let wei = parse_eth_to_wei(s).unwrap();
            assert_eq!(format_wei_as_eth(wei), s, "round-trip drifted for {s}");
        }
    }
}
