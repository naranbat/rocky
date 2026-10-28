use std::fmt;

use regex::Regex;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::error::Error;
use crate::extract::text;
use crate::pattern::Pattern;
use crate::vars;

/// A job branch condition: either `this` with one or more operators (all must hold;
/// `exists` only alone), or exactly one of `all`, `any`, `not`. A list is shorthand for `all`.
/// Operator values other than `matches` may use `${VAR}`, expanded when the condition is evaluated.
#[derive(Clone, Debug)]
pub struct Condition(Fields);

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    #[serde(default, deserialize_with = "vars::var_path")]
    this: Option<String>,
    #[serde(default, deserialize_with = "present")]
    gt: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    gte: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    lt: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    lte: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    eq: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    ne: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    contains: Option<Value>,
    matches: Option<Pattern>,
    exists: Option<bool>,
    all: Option<Vec<Condition>>,
    any: Option<Vec<Condition>>,
    not: Option<Box<Condition>>,
}

/// Reads an operator's value, keeping a written `null` as `Some(Value::Null)`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// Picks the shape by YAML type, so a mistake inside a map reports the real cause.
impl<'de> Deserialize<'de> for Condition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ConditionVisitor;

        impl<'de> Visitor<'de> for ConditionVisitor {
            type Value = Condition;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a condition map or a list of conditions")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Condition, A::Error> {
                let all = Vec::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(Condition(Fields {
                    all: Some(all),
                    ..Fields::default()
                }))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Condition, A::Error> {
                Fields::deserialize(de::value::MapAccessDeserializer::new(map)).map(Condition)
            }
        }

        deserializer.deserialize_any(ConditionVisitor)
    }
}

enum Kind<'a> {
    All(&'a [Condition]),
    Any(&'a [Condition]),
    Not(&'a Condition),
    Compare { var: &'a str, ops: Vec<Op<'a>> },
}

enum Op<'a> {
    Gt(&'a Value),
    Gte(&'a Value),
    Lt(&'a Value),
    Lte(&'a Value),
    Eq(&'a Value),
    Ne(&'a Value),
    Contains(&'a Value),
    Matches(&'a Regex),
    Exists(bool),
}

impl Condition {
    /// Checks the shape and threshold operands, recursively.
    pub fn validate(&self) -> Result<(), Error> {
        match self.kind()? {
            Kind::All(conditions) | Kind::Any(conditions) => {
                conditions.iter().try_for_each(Condition::validate)
            }
            Kind::Not(condition) => condition.validate(),
            Kind::Compare { .. } => {
                for (name, operand) in self.thresholds() {
                    if let Some(value) = operand {
                        let template = matches!(value, Value::String(s) if s.contains("${"));
                        if !template && as_number(value).is_none() {
                            return Err(Error::InvalidCondition(format!(
                                "`{name}` needs a number, got {value}"
                            )));
                        }
                    }
                }
                Ok(())
            }
        }
    }

    pub fn evaluate(&self, lookup: &dyn Fn(&str) -> Option<Value>) -> Result<bool, Error> {
        match self.kind()? {
            Kind::All(conditions) => {
                for condition in conditions {
                    if !condition.evaluate(lookup)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Kind::Any(conditions) => {
                for condition in conditions {
                    if condition.evaluate(lookup)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Kind::Not(condition) => Ok(!condition.evaluate(lookup)?),
            Kind::Compare { var, ops } => {
                let Some(left) = lookup(var) else {
                    return match ops.as_slice() {
                        [Op::Exists(expected)] => Ok(!expected),
                        _ => Err(Error::UndefinedVar(var.to_string())),
                    };
                };
                for op in &ops {
                    if !holds(var, &left, op, lookup)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    fn kind(&self) -> Result<Kind<'_>, Error> {
        let f = &self.0;
        let combinators = [f.all.is_some(), f.any.is_some(), f.not.is_some()]
            .into_iter()
            .filter(|set| *set)
            .count();
        let mut ops = Vec::new();
        if let Some(value) = &f.gt {
            ops.push(Op::Gt(value));
        }
        if let Some(value) = &f.gte {
            ops.push(Op::Gte(value));
        }
        if let Some(value) = &f.lt {
            ops.push(Op::Lt(value));
        }
        if let Some(value) = &f.lte {
            ops.push(Op::Lte(value));
        }
        if let Some(value) = &f.eq {
            ops.push(Op::Eq(value));
        }
        if let Some(value) = &f.ne {
            ops.push(Op::Ne(value));
        }
        if let Some(value) = &f.contains {
            ops.push(Op::Contains(value));
        }
        if let Some(regex) = f.matches.as_deref() {
            ops.push(Op::Matches(regex));
        }
        if let Some(expected) = f.exists {
            ops.push(Op::Exists(expected));
        }

        let exists_mixed = f.exists.is_some() && ops.len() > 1;
        match (&f.this, combinators, ops.len()) {
            (Some(var), 0, 1..) if !exists_mixed => Ok(Kind::Compare { var, ops }),
            (None, 1, 0) => {
                if let Some(conditions) = &f.all {
                    Ok(Kind::All(conditions))
                } else if let Some(conditions) = &f.any {
                    Ok(Kind::Any(conditions))
                } else {
                    f.not.as_deref().map(Kind::Not).ok_or_else(shape_error)
                }
            }
            _ => Err(shape_error()),
        }
    }

    fn thresholds(&self) -> [(&'static str, &Option<Value>); 4] {
        [
            ("gt", &self.0.gt),
            ("gte", &self.0.gte),
            ("lt", &self.0.lt),
            ("lte", &self.0.lte),
        ]
    }
}

fn shape_error() -> Error {
    Error::InvalidCondition(
        "expected `this` with one or more of gt, gte, lt, lte, eq, ne, contains, matches, \
         or `exists` alone; or exactly one of all, any, not; or a list"
            .to_string(),
    )
}

/// Whether one operator holds for the defined value `left` of `var`.
fn holds(
    var: &str,
    left: &Value,
    op: &Op,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Result<bool, Error> {
    Ok(match *op {
        Op::Gt(right) => number(var, left)? > threshold("gt", right, lookup)?,
        Op::Gte(right) => number(var, left)? >= threshold("gte", right, lookup)?,
        Op::Lt(right) => number(var, left)? < threshold("lt", right, lookup)?,
        Op::Lte(right) => number(var, left)? <= threshold("lte", right, lookup)?,
        Op::Eq(right) => loose_eq(left, &expand(right, lookup)?),
        Op::Ne(right) => !loose_eq(left, &expand(right, lookup)?),
        Op::Contains(right) => contains(var, left, &expand(right, lookup)?)?,
        Op::Matches(regex) => regex.is_match(&text(left)),
        Op::Exists(expected) => expected,
    })
}

/// A number, or a string holding a finite number.
fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

/// An integer, or a string holding one, kept exact beyond f64's 2^53.
fn as_integer(value: &Value) -> Option<i128> {
    match value {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// The operator value with its `${VAR}`s expanded.
fn expand(value: &Value, lookup: &dyn Fn(&str) -> Option<Value>) -> Result<Value, Error> {
    let mut value = value.clone();
    vars::substitute_value(&mut value, lookup)?;
    Ok(value)
}

fn threshold(
    name: &str,
    value: &Value,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Result<f64, Error> {
    let value = expand(value, lookup)?;
    as_number(&value)
        .ok_or_else(|| Error::Condition(format!("`{name}` needs a number, got {value}")))
}

fn number(var: &str, value: &Value) -> Result<f64, Error> {
    as_number(value).ok_or_else(|| Error::Condition(format!("`{var}` is not a number ({value})")))
}

/// Text when both sides are strings, numeric when both are numeric, text when either is a
/// string, else JSON equality.
pub(crate) fn loose_eq(left: &Value, right: &Value) -> bool {
    if let (Value::String(a), Value::String(b)) = (left, right) {
        return a == b;
    }
    if let (Some(a), Some(b)) = (as_integer(left), as_integer(right)) {
        return a == b;
    }
    if let (Some(a), Some(b)) = (as_number(left), as_number(right)) {
        return a == b;
    }
    if left.is_string() || right.is_string() {
        return text(left) == text(right);
    }
    left == right
}

fn contains(var: &str, left: &Value, right: &Value) -> Result<bool, Error> {
    match left {
        Value::String(s) => Ok(s.contains(&text(right))),
        Value::Array(items) => Ok(items.iter().any(|item| loose_eq(item, right))),
        Value::Object(map) => Ok(map.contains_key(&text(right))),
        other => Err(Error::Condition(format!(
            "`{var}` must be a string, array or object for `contains` ({other})"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cond(yaml: &str) -> Condition {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn lookup(name: &str) -> Option<Value> {
        match name {
            "N" => Some(json!(15)),
            "NS" => Some(json!("15")),
            "STATUS" => Some(json!(200)),
            "MSG" => Some(json!("warning: disk low")),
            "TAGS" => Some(json!(["a", "b", 3])),
            "OBJ" => Some(json!({"id": 1})),
            "FLAG" => Some(json!(true)),
            "WORD" => Some(json!("abc")),
            "SUB" => Some(json!("disk")),
            "TAG" => Some(json!("b")),
            "NONE" => Some(Value::Null),
            _ => None,
        }
    }

    fn eval(yaml: &str) -> bool {
        let condition = cond(yaml);
        condition.validate().unwrap();
        condition.evaluate(&lookup).unwrap()
    }

    #[test]
    fn thresholds() {
        assert!(eval("{this: N, gt: 10}"));
        assert!(!eval("{this: N, gt: 15}"));
        assert!(eval("{this: N, gte: 15}"));
        assert!(eval("{this: N, lt: 15.5}"));
        assert!(eval("{this: N, lte: 15}"));
        assert!(!eval("{this: N, lt: 10}"));
    }

    #[test]
    fn numeric_strings_compare_as_numbers() {
        assert!(eval("{this: NS, gt: 10}"));
        assert!(eval("{this: NS, eq: 15}"));
        assert!(eval("{this: STATUS, eq: \"200\"}"));
        assert!(eval("{this: STATUS, eq: 200.0}"));
        assert!(eval("{this: N, gt: \"10\"}"));
        assert!(eval("{this: NS, lte: ' 15 '}"));
        assert!(!eval("{this: N, lt: \"10\"}"));
    }

    #[test]
    fn equality_and_inequality() {
        assert!(eval("{this: WORD, eq: abc}"));
        assert!(eval("{this: WORD, ne: abd}"));
        assert!(eval("{this: FLAG, eq: true}"));
        assert!(!eval("{this: FLAG, eq: false}"));
        assert!(eval("{this: OBJ, eq: {id: 1}}"));
        assert!(!eval("{this: STATUS, ne: 200}"));
    }

    #[test]
    fn loose_eq_numbers_and_strings() {
        assert!(!loose_eq(&json!("1.1"), &json!("1.10")));
        assert!(!loose_eq(&json!("007"), &json!("7")));
        assert!(loose_eq(&json!(5), &json!("5")));
        assert!(loose_eq(&json!(1), &json!(1.0)));
        let big = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert!(!loose_eq(
            &big("9007199254740993"),
            &big("9007199254740992")
        ));
        assert!(!loose_eq(
            &big("9007199254740993"),
            &json!("9007199254740992")
        ));
    }

    #[test]
    fn null_operands_are_kept() {
        assert!(eval("{this: NONE, eq: null}"));
        assert!(!eval("{this: WORD, eq: null}"));
        assert!(eval("{this: WORD, ne: null}"));
        assert!(!eval("{this: N, gt: 0, eq: null}"));
    }

    #[test]
    fn evaluation_errors() {
        for (yaml, expected) in [
            (
                "{this: MISSING, eq: 1}",
                "variable `MISSING` is not defined",
            ),
            (
                "{this: MISSING, gt: 1, lt: 5}",
                "variable `MISSING` is not defined",
            ),
            (
                "{this: N, gt: '${MISSING}'}",
                "variable `MISSING` is not defined",
            ),
            ("{this: WORD, gt: 1}", "`WORD` is not a number"),
            (
                "{this: N, gt: '${WORD}'}",
                "`gt` needs a number, got \"abc\"",
            ),
            ("{this: N, contains: 1}", "for `contains`"),
        ] {
            let err = cond(yaml).evaluate(&lookup).unwrap_err();
            assert!(err.to_string().contains(expected), "{yaml}: {err}");
        }
    }

    #[test]
    fn contains_string_array_and_object() {
        assert!(eval("{this: MSG, contains: disk}"));
        assert!(!eval("{this: MSG, contains: error}"));
        assert!(eval("{this: TAGS, contains: b}"));
        assert!(eval("{this: TAGS, contains: 3}"));
        assert!(!eval("{this: TAGS, contains: c}"));
        assert!(eval("{this: OBJ, contains: id}"));
    }

    #[test]
    fn regex_matches() {
        assert!(eval("{this: MSG, matches: '^warn'}"));
        assert!(eval("{this: MSG, matches: 'disk\\s+low$'}"));
        assert!(!eval("{this: MSG, matches: '^disk'}"));
        assert!(eval("{this: N, matches: '^1\\d$'}"));
    }

    #[test]
    fn combinators() {
        assert!(eval(
            "{all: [{this: N, gt: 10}, {this: MSG, contains: warn}]}"
        ));
        assert!(!eval(
            "{all: [{this: N, gt: 10}, {this: MSG, contains: nope}]}"
        ));
        assert!(eval("{any: [{this: N, gt: 99}, {this: WORD, eq: abc}]}"));
        assert!(!eval("{any: []}"));
        assert!(eval("{all: []}"));
        assert!(eval("{not: {this: N, lt: 10}}"));
        assert!(eval(
            "{all: [{not: {this: MSG, matches: '^ignore'}}, {any: [{this: NS, eq: 15}]}]}"
        ));
    }

    #[test]
    fn several_operators_must_all_hold() {
        assert!(eval("{this: N, gt: 10, lt: 20}"));
        assert!(!eval("{this: N, gt: 15, lt: 20}"));
        assert!(!eval("{this: N, gt: 10, lt: 15}"));
        assert!(eval("{this: MSG, contains: disk, matches: '^warn'}"));
        assert!(!eval(
            "{this: MSG, contains: disk, ne: 'warning: disk low'}"
        ));
    }

    #[test]
    fn list_is_all() {
        assert!(eval("[{this: N, gt: 10}, {this: MSG, contains: warn}]"));
        assert!(!eval("[{this: N, gt: 10}, {this: MSG, contains: nope}]"));
        assert!(eval("[]"));
        assert!(eval(
            "{any: [[{this: N, gt: 99}], [{this: WORD, eq: abc}]]}"
        ));
        assert!(!eval("{not: [{this: N, lt: 99}]}"));
    }

    #[test]
    fn short_circuits_before_undefined_var() {
        assert!(!eval("{all: [{this: N, gt: 99}, {this: MISSING, eq: 1}]}"));
        assert!(eval("{any: [{this: N, gt: 1}, {this: MISSING, eq: 1}]}"));
    }

    #[test]
    fn exists_checks_definition_without_erroring() {
        assert!(eval("{this: N, exists: true}"));
        assert!(!eval("{this: N, exists: false}"));
        assert!(eval("{this: NONE, exists: true}"));
        assert!(!eval("{this: MISSING, exists: true}"));
        assert!(eval("{this: MISSING, exists: false}"));
        assert!(eval(
            "{any: [{this: MISSING, exists: false}, {this: MISSING, eq: 1}]}"
        ));
    }

    #[test]
    fn operator_values_expand_vars() {
        assert!(eval("{this: STATUS, gt: '${N}'}"));
        assert!(!eval("{this: N, gt: '${STATUS}'}"));
        assert!(eval("{this: N, gte: '${NS}'}"));
        assert!(eval("{this: N, lt: '${N}0'}"));
        assert!(eval("{this: WORD, eq: '${WORD}'}"));
        assert!(eval("{this: OBJ, eq: '${OBJ}'}"));
        assert!(!eval("{this: WORD, ne: '${WORD}'}"));
        assert!(eval("{this: MSG, contains: '${SUB}'}"));
        assert!(eval("{this: TAGS, contains: '${TAG}'}"));
        assert!(!eval("{this: WORD, eq: '$${WORD}'}"));
    }

    #[test]
    fn this_may_be_written_with_braces() {
        assert!(eval("{this: '${N}', gt: 10}"));
        assert!(eval("[{this: '${MSG}', contains: disk}]"));
    }

    #[test]
    fn invalid_shapes_rejected() {
        for yaml in [
            "{this: N}",
            "{this: N, exists: true, eq: 1}",
            "{this: N, exists: false, gt: 1}",
            "[{this: N}]",
            "{exists: true}",
            "{gt: 1}",
            "{this: N, all: []}",
            "{all: [], any: []}",
            "{this: N, gt: ten}",
            "{this: N, gt: null}",
            "{all: [{this: N, gt: ten}]}",
        ] {
            assert!(
                matches!(cond(yaml).validate(), Err(Error::InvalidCondition(_))),
                "{yaml}"
            );
        }
    }

    #[test]
    fn unknown_key_rejected() {
        assert!(serde_yaml::from_str::<Condition>("{this: N, greater: 1}").is_err());
        assert!(serde_yaml::from_str::<Condition>("{this: N, exists: maybe}").is_err());
        assert!(serde_yaml::from_str::<Condition>("{var: N, eq: 1}").is_err());
    }

    #[test]
    fn invalid_regex_fails_to_parse() {
        for yaml in [
            "{this: N, matches: '('}",
            "{not: {this: N, matches: '('}}",
            "[{this: N, matches: '('}]",
        ] {
            let err = serde_yaml::from_str::<Condition>(yaml).unwrap_err();
            assert!(
                err.to_string().contains("invalid regex `(`"),
                "{yaml}: {err}"
            );
        }
    }

    #[test]
    fn rejects_a_condition_that_is_neither_a_map_nor_a_list() {
        let err = serde_yaml::from_str::<Condition>("5").unwrap_err();
        assert!(
            err.to_string()
                .contains("a condition map or a list of conditions"),
            "{err}"
        );
    }
}
