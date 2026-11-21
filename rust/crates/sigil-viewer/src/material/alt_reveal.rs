//! `pbr.vfx`'s alt albedo: `g_tAltColor_Reveal` holds a second albedo in RGB and a reveal
//! mask in alpha, and `g_flAltTextureAmount` wipes it over `g_tColor`. Violet's brush is inked
//! this way while she has ammo; Solomon's pieces turn black the same way. Ported from
//! Grimoire (MIT, `src/lib/source2AltReveal.ts`, `src/lib/dynamicScalar.ts`); the equation is
//! the shipped pixel shader's (vpkmerge `docs/source2-preview-data-contract.md`).
//!
//! The amount is often an expression (`max($AmmoPercent, smoothstep(0.5, 1,
//! $ReloadProgress))`). It is evaluated once, for a hero standing in the preview: a full
//! magazine, no reload under way. An expression this evaluator cannot read keeps the
//! material's authored static amount.

/// What the engine reports for a hero at rest in the preview.
pub const REST_ATTRIBUTES: &[(&str, f64)] = &[("$AmmoPercent", 1.0), ("$ReloadProgress", 0.0)];

/// The shader's default reveal softness; its authoring range is 0 to 0.5.
pub const DEFAULT_SOFTNESS: f32 = 0.05;

/// How much of the alt albedo one texel shows, for a reveal mask value in 0..1. The same
/// function the scene shader runs per pixel.
pub fn factor(amount: f32, softness: f32, mask: f32) -> f32 {
    if amount <= 0.0 {
        return 0.0;
    }
    if amount >= 1.0 {
        return 1.0;
    }
    let threshold = mask.clamp(1e-4, 0.9999) * (1.0 - 2.0 * softness) + softness;
    let distance = amount - threshold;
    if softness <= 0.0 {
        return if distance > 0.0 { 1.0 } else { 0.0 };
    }
    let t = ((distance + softness) / (2.0 * softness)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Evaluates a decompiled scalar expression at rest (`time` is 0), or None for anything
/// outside Grimoire's tier 1: literals, `time()`, sin, cos, abs, floor, frac, saturate, min,
/// max, clamp, smoothstep, `+ - * /`, unary minus, parentheses and the rest attributes.
pub fn eval_at_rest(source: &str) -> Option<f64> {
    let tokens = tokenize(source)?;
    let mut p = Parser { tokens, at: 0 };
    let v = p.expr()?;
    (p.at == p.tokens.len() && v.is_finite()).then_some(v)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(src: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if "+-*/(),".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // A trailing `f` suffix (`0.5f`) is a float literal too.
            let text: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == 'f' {
                i += 1;
            }
            out.push(Tok::Num(text.parse().ok()?));
        } else if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            return None;
        }
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }
    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') {
                v *= self.unary()?;
            } else if self.eat('/') {
                v /= self.unary()?;
            } else {
                return Some(v);
            }
        }
    }
    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return self.unary().map(|v| -v);
        }
        self.atom()
    }
    fn atom(&mut self) -> Option<f64> {
        let tok = self.peek()?.clone();
        self.at += 1;
        match tok {
            Tok::Num(n) => Some(n),
            Tok::Op('(') => {
                let v = self.expr()?;
                self.eat(')').then_some(v)
            }
            Tok::Ident(name) if name.starts_with('$') => REST_ATTRIBUTES.iter().find(|(a, _)| a.eq_ignore_ascii_case(&name)).map(|(_, v)| *v),
            Tok::Ident(name) => {
                let name = name.to_ascii_lowercase();
                if !self.eat('(') {
                    return (name == "time").then_some(0.0);
                }
                let mut args = Vec::new();
                if !self.eat(')') {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(')') {
                            break;
                        }
                        if !self.eat(',') {
                            return None;
                        }
                    }
                }
                call(&name, &args)
            }
            Tok::Op(_) => None,
        }
    }
}

fn call(name: &str, a: &[f64]) -> Option<f64> {
    Some(match (name, a) {
        ("time", []) => 0.0,
        ("sin", [x]) => x.sin(),
        ("cos", [x]) => x.cos(),
        ("abs", [x]) => x.abs(),
        ("floor", [x]) => x.floor(),
        ("frac", [x]) => x - x.floor(),
        ("saturate", [x]) => x.clamp(0.0, 1.0),
        ("min", [x, y]) => x.min(*y),
        ("max", [x, y]) => x.max(*y),
        ("clamp", [x, lo, hi]) => x.max(*lo).min(*hi),
        ("smoothstep", [e0, e1, x]) => {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn violets_brush_is_inked_at_rest() {
        assert_eq!(eval_at_rest("max($AmmoPercent,smoothstep(0.5,1,$ReloadProgress))"), Some(1.0));
    }

    #[test]
    fn tier_one_math_evaluates() {
        assert_eq!(eval_at_rest("0.5 * sin(3 * time()) + 0.5"), Some(0.5));
        assert_eq!(eval_at_rest("-(2 - 3) / 4"), Some(0.25));
        assert_eq!(eval_at_rest("clamp(2, 0, 1) + saturate(-1)"), Some(1.0));
    }

    #[test]
    fn unknown_attributes_and_syntax_fall_back() {
        assert_eq!(eval_at_rest("$Health"), None);
        assert_eq!(eval_at_rest("float2(1, 2)"), None);
        assert_eq!(eval_at_rest("1 > 0"), None);
        assert_eq!(eval_at_rest("max(1"), None);
    }

    #[test]
    fn reveal_factor_matches_the_shader() {
        assert_eq!(factor(0.0, 0.05, 0.5), 0.0);
        assert_eq!(factor(1.0, 0.05, 0.99), 1.0);
        assert!(factor(0.5, 0.05, 0.1) > 0.99, "a low mask shows the alt color early");
        assert!(factor(0.5, 0.05, 0.9) < 0.01, "a high mask keeps the base color");
        assert_eq!(factor(0.5, 0.0, 0.3), 1.0);
        assert_eq!(factor(0.5, 0.0, 0.7), 0.0);
    }
}
