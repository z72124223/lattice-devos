use lattice_task_domain::{
    MAX_CANONICAL_DECIMAL_BYTES, MAX_CANONICAL_DECIMAL_INTEGER_DIGITS, MAX_CANONICAL_DECIMAL_SCALE,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecimalLimit {
    Within,
    Exceeded,
    Invalid,
}

pub(crate) fn is_zero(value: &str) -> Result<bool, ()> {
    let decimal = Decimal::parse(value).ok_or(())?;
    Ok(decimal.digits.iter().all(|digit| *digit == 0))
}

pub(crate) fn checked_sum_within(used: &str, requested: &str, limit: &str) -> DecimalLimit {
    let Some(used) = Decimal::parse(used) else {
        return DecimalLimit::Invalid;
    };
    let Some(requested) = Decimal::parse(requested) else {
        return DecimalLimit::Invalid;
    };
    let Some(limit) = Decimal::parse(limit) else {
        return DecimalLimit::Invalid;
    };

    let scale = used.scale.max(requested.scale).max(limit.scale);
    let Some(left) = used.scaled_digits(scale) else {
        return DecimalLimit::Invalid;
    };
    let Some(right) = requested.scaled_digits(scale) else {
        return DecimalLimit::Invalid;
    };
    let Some(limit) = limit.scaled_digits(scale) else {
        return DecimalLimit::Invalid;
    };
    let Some(sum) = add_digits(&left, &right) else {
        return DecimalLimit::Invalid;
    };
    if compare_digits(&sum, &limit).is_gt() {
        DecimalLimit::Exceeded
    } else {
        DecimalLimit::Within
    }
}

struct Decimal {
    digits: Vec<u8>,
    scale: usize,
}

impl Decimal {
    fn parse(value: &str) -> Option<Self> {
        if value.is_empty() || value.len() > MAX_CANONICAL_DECIMAL_BYTES {
            return None;
        }
        let mut pieces = value.split('.');
        let integer = pieces.next()?;
        let fraction = pieces.next();
        if pieces.next().is_some()
            || integer.is_empty()
            || integer.len() > MAX_CANONICAL_DECIMAL_INTEGER_DIGITS
            || !integer.bytes().all(|byte| byte.is_ascii_digit())
            || (integer.len() > 1 && integer.starts_with('0'))
        {
            return None;
        }
        let fraction = fraction.unwrap_or("");
        if (!fraction.is_empty()
            && (fraction.len() > MAX_CANONICAL_DECIMAL_SCALE
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
                || fraction.ends_with('0')))
            || (value.contains('.') && fraction.is_empty())
        {
            return None;
        }
        let mut digits = Vec::with_capacity(integer.len() + fraction.len());
        digits.extend(integer.bytes().map(|byte| byte - b'0'));
        digits.extend(fraction.bytes().map(|byte| byte - b'0'));
        Some(Self {
            digits,
            scale: fraction.len(),
        })
    }

    fn scaled_digits(&self, scale: usize) -> Option<Vec<u8>> {
        let padding = scale.checked_sub(self.scale)?;
        let capacity = self.digits.len().checked_add(padding)?;
        if capacity > MAX_CANONICAL_DECIMAL_BYTES {
            return None;
        }
        let mut digits = self.digits.clone();
        digits.resize(capacity, 0);
        trim_leading_zeroes(&mut digits);
        Some(digits)
    }
}

fn add_digits(left: &[u8], right: &[u8]) -> Option<Vec<u8>> {
    let width = left.len().max(right.len());
    if width > MAX_CANONICAL_DECIMAL_BYTES {
        return None;
    }
    let mut output = Vec::with_capacity(width + 1);
    let mut carry = 0_u8;
    for offset in 0..width {
        let left = left
            .len()
            .checked_sub(offset + 1)
            .map_or(0, |index| left[index]);
        let right = right
            .len()
            .checked_sub(offset + 1)
            .map_or(0, |index| right[index]);
        let total = left + right + carry;
        output.push(total % 10);
        carry = total / 10;
    }
    if carry != 0 {
        output.push(carry);
    }
    if output.len() > MAX_CANONICAL_DECIMAL_BYTES {
        return None;
    }
    output.reverse();
    trim_leading_zeroes(&mut output);
    Some(output)
}

fn compare_digits(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn trim_leading_zeroes(digits: &mut Vec<u8>) {
    let leading = digits
        .iter()
        .take_while(|digit| **digit == 0)
        .count()
        .min(digits.len().saturating_sub(1));
    if leading > 0 {
        digits.drain(..leading);
    }
}

#[cfg(test)]
mod tests {
    use super::{DecimalLimit, checked_sum_within, is_zero};

    #[test]
    fn compares_exact_decimal_strings_without_floats() {
        assert_eq!(
            checked_sum_within("0.1", "0.2", "0.3"),
            DecimalLimit::Within
        );
        assert_eq!(
            checked_sum_within("0.1", "0.21", "0.3"),
            DecimalLimit::Exceeded
        );
        assert_eq!(
            checked_sum_within("18446744073709551616", "1", "18446744073709551617"),
            DecimalLimit::Within
        );
        assert_eq!(checked_sum_within("01", "0", "1"), DecimalLimit::Invalid);
        assert_eq!(is_zero("0"), Ok(true));
        assert_eq!(is_zero("0.01"), Ok(false));

        let maximum_integer = "8".repeat(127);
        let maximum_scale = format!("0.{}1", "0".repeat(127));
        let maximum_limit = format!("{}.{}", "9".repeat(127), "9".repeat(128));
        assert_eq!(
            checked_sum_within(&maximum_integer, &maximum_scale, &maximum_limit),
            DecimalLimit::Within
        );
        assert_eq!(is_zero(&"9".repeat(128)), Err(()));
        assert_eq!(is_zero(&format!("0.{}", "9".repeat(129))), Err(()));
        assert_eq!(is_zero(&"9".repeat(257)), Err(()));
    }
}
