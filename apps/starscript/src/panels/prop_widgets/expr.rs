//! The arithmetic typed into a numeric field.

/// Evaluates what was typed into a numeric field: a number, or simple arithmetic with
/// `+ - * /` and parentheses (`2716+88`). A leading `+`, `*` or `/` applies to `current`
/// (`*2`, `/3`). Returns `None` for anything that does not give a finite number.
pub(super) fn eval_expression(text: &str, current: f64) -> Option<f64> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text.replace('×', "*").replace('÷', "/").replace('−', "-");
    let relative = matches!(text.chars().next()?, '*' | '/' | '+');
    let source = if relative {
        format!("({current}){text}")
    } else {
        text
    };
    let mut parser = Parser {
        chars: source.chars().collect(),
        at: 0,
    };
    let value = parser.sum()?;
    (parser.at == parser.chars.len() && value.is_finite()).then_some(value)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.at += 1;
            let rhs = self.product()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    fn product(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        while let Some(op @ ('*' | '/')) = self.peek() {
            self.at += 1;
            let rhs = self.unary()?;
            if op == '/' && rhs == 0.0 {
                return None;
            }
            value = if op == '*' { value * rhs } else { value / rhs };
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.at += 1;
                self.unary().map(|v| -v)
            }
            '(' => {
                self.at += 1;
                let value = self.sum()?;
                if self.peek()? != ')' {
                    return None;
                }
                self.at += 1;
                Some(value)
            }
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        while let Some(c) = self.peek() {
            // A sign is part of the number only as an exponent's, as in the `1e-3` that
            // formatting a small value gives.
            let exponent_sign = matches!(c, '-' | '+')
                && self.at > start
                && matches!(self.chars[self.at - 1], 'e' | 'E');
            if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E') || exponent_sign {
                self.at += 1;
            } else {
                break;
            }
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_evaluates_sums_and_products() {
        assert_eq!(eval_expression("2716+88", 0.0), Some(2804.0));
        assert_eq!(eval_expression("2 + 3 * 4", 0.0), Some(14.0));
        assert_eq!(eval_expression("(2+3)*4", 0.0), Some(20.0));
        assert_eq!(eval_expression("-5", 0.0), Some(-5.0));
        assert_eq!(eval_expression("10--2", 0.0), Some(12.0));
        assert_eq!(eval_expression("1.5", 0.0), Some(1.5));
        assert_eq!(eval_expression("1e3", 0.0), Some(1000.0));
    }

    #[test]
    fn leading_operators_apply_to_the_current_value() {
        assert_eq!(eval_expression("*2", 21.0), Some(42.0));
        assert_eq!(eval_expression("/3", 90.0), Some(30.0));
        assert_eq!(eval_expression("+8", 2.0), Some(10.0));
        assert_eq!(eval_expression("*2+1", 5.0), Some(11.0));
        assert_eq!(eval_expression("*2", -4.0), Some(-8.0));
    }

    #[test]
    fn bad_input_is_rejected() {
        for bad in [
            "", "  ", "abc", "1+", "*", "(1+2", "1+2)", "1/0", "/0", "1..2", "2,3", "--", "1e999",
        ] {
            assert_eq!(eval_expression(bad, 4.0), None, "{bad:?}");
        }
    }
}
