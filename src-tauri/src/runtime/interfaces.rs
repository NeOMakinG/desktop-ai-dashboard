//! Native validation for inert shared presentation. No HTML, code, URLs or actions.
use crate::types::*;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
fn object<'a>(v: &'a Value, keys: &[&str]) -> AppResult<&'a Map<String, Value>> {
    let o = v.as_object().ok_or_else(AppError::invalid)?;
    if o.keys()
        .any(|k| !keys.contains(&k.as_str()) || forbidden(k))
    {
        return Err(AppError::invalid());
    }
    Ok(o)
}
fn forbidden(s: &str) -> bool {
    matches!(s, "__proto__" | "constructor" | "prototype")
}
fn text(v: &Value, max: usize) -> AppResult<&str> {
    let s = v.as_str().ok_or_else(AppError::invalid)?;
    if s.chars().count() > max
        || s.chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(AppError::invalid());
    }
    Ok(s)
}
fn id(v: &Value) -> AppResult<&str> {
    let s = text(v, 80)?;
    if s.is_empty()
        || forbidden(s)
        || !s.as_bytes()[0].is_ascii_alphanumeric()
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(AppError::invalid());
    }
    Ok(s)
}
fn array(v: &Value, max: usize) -> AppResult<&Vec<Value>> {
    let a = v.as_array().ok_or_else(AppError::invalid)?;
    if a.len() > max {
        return Err(AppError::invalid());
    }
    Ok(a)
}
fn integer(v: &Value, min: u64, max: u64) -> AppResult<u64> {
    v.as_u64()
        .filter(|n| *n >= min && *n <= max)
        .ok_or_else(AppError::invalid)
}
fn instant(v: &Value) -> AppResult<chrono::DateTime<chrono::FixedOffset>> {
    let s = text(v, 40)?;
    if !s.ends_with('Z') {
        return Err(AppError::invalid());
    }
    chrono::DateTime::parse_from_rfc3339(s).map_err(|_| AppError::invalid())
}
struct Dataset<'a> {
    fields: HashMap<&'a str, &'a str>,
    rows: &'a Vec<Value>,
}
struct Check<'a> {
    datasets: HashMap<&'a str, Dataset<'a>>,
    nodes: HashSet<String>,
    content: usize,
}
impl Check<'_> {
    fn node(&mut self, v: &Value, depth: usize) -> AppResult<()> {
        let node_id = id(&v["id"])?;
        if self.nodes.len() >= 40 || !self.nodes.insert(node_id.to_owned()) {
            return Err(AppError::invalid());
        }
        let kind = v["type"].as_str().ok_or_else(AppError::invalid)?;
        match kind {
            "stack" | "grid" | "card" => {
                let keys: &[&str] = match kind {
                    "grid" => &["id", "type", "columns", "children"],
                    "card" => &["id", "type", "title", "children"],
                    _ => &["id", "type", "children"],
                };
                object(v, keys)?;
                if depth >= 3 {
                    return Err(AppError::invalid());
                }
                if kind == "grid" {
                    integer(&v["columns"], 1, 4)?;
                }
                if kind == "card" {
                    text(&v["title"], 160)?;
                }
                for child in array(&v["children"], 40)? {
                    self.node(child, depth + 1)?;
                }
            }
            "text" => {
                object(v, &["id", "type", "text"])?;
                text(&v["text"], 2000)?;
                self.content += 1;
            }
            "metric" | "table" | "chart" => {
                self.content += 1;
                let data = self
                    .datasets
                    .get(id(&v["datasetId"])?)
                    .ok_or_else(AppError::invalid)?;
                let field = |v: &Value, ty: Option<&str>| -> AppResult<String> {
                    let name = id(v)?;
                    let actual = data.fields.get(name).ok_or_else(AppError::invalid)?;
                    if ty.is_some_and(|ty| *actual != ty) {
                        return Err(AppError::invalid());
                    }
                    Ok(name.to_owned())
                };
                match kind {
                    "metric" => {
                        object(
                            v,
                            &[
                                "id",
                                "type",
                                "label",
                                "datasetId",
                                "field",
                                "format",
                                "currency",
                            ],
                        )?;
                        text(&v["label"], 160)?;
                        field(&v["field"], Some("number"))?;
                        if data.rows.len() > 1 {
                            return Err(AppError::invalid());
                        }
                        match v["format"].as_str() {
                            Some("currency") => {
                                let s = text(&v["currency"], 3)?;
                                if s.len() != 3 || !s.bytes().all(|b| b.is_ascii_uppercase()) {
                                    return Err(AppError::invalid());
                                }
                            }
                            Some("number" | "percent") if v.get("currency").is_none() => (),
                            _ => return Err(AppError::invalid()),
                        }
                    }
                    "table" => {
                        object(v, &["id", "type", "datasetId", "columns"])?;
                        let columns = array(&v["columns"], 12)?;
                        if columns.is_empty() {
                            return Err(AppError::invalid());
                        }
                        let mut used = HashSet::new();
                        for column in columns {
                            object(column, &["field", "label"])?;
                            if !used.insert(field(&column["field"], None)?) {
                                return Err(AppError::invalid());
                            }
                            text(&column["label"], 160)?;
                        }
                    }
                    _ => {
                        object(
                            v,
                            &[
                                "id",
                                "type",
                                "kind",
                                "datasetId",
                                "xField",
                                "yField",
                                "label",
                                "units",
                            ],
                        )?;
                        let line = match v["kind"].as_str() {
                            Some("line") => true,
                            Some("bar") => false,
                            _ => return Err(AppError::invalid()),
                        };
                        let x = field(&v["xField"], Some(if line { "timestamp" } else { "text" }))?;
                        field(&v["yField"], Some("number"))?;
                        text(&v["label"], 160)?;
                        text(&v["units"], 80)?;
                        let mut seen = HashSet::new();
                        let mut previous = None;
                        for row in data.rows {
                            let value = row[&x].as_str().ok_or_else(AppError::invalid)?;
                            if !seen.insert(value) {
                                return Err(AppError::invalid());
                            }
                            if line {
                                let time = instant(&row[&x])?;
                                if previous.is_some_and(|prev| time <= prev) {
                                    return Err(AppError::invalid());
                                }
                                previous = Some(time);
                            }
                        }
                    }
                }
            }
            _ => return Err(AppError::invalid()),
        }
        if self.content > 12 {
            return Err(AppError::invalid());
        }
        Ok(())
    }
}
pub fn validate(spec: &Value) -> AppResult<()> {
    if serde_json::to_vec(spec)
        .map_err(|_| AppError::invalid())?
        .len()
        > 65_536
    {
        return Err(AppError::invalid());
    }
    object(spec, &["schemaVersion", "kind", "root", "datasets"])?;
    if spec["schemaVersion"] != 1 || spec["kind"] != "components" {
        return Err(AppError::invalid());
    }
    let mut datasets = HashMap::new();
    for data in array(&spec["datasets"], 12)? {
        object(data, &["id", "columns", "rows"])?;
        let data_id = id(&data["id"])?;
        let mut fields = HashMap::new();
        for column in array(&data["columns"], 12)? {
            object(column, &["id", "type"])?;
            let name = id(&column["id"])?;
            let kind = text(&column["type"], 12)?;
            if !matches!(kind, "text" | "number" | "timestamp")
                || fields.insert(name, kind).is_some()
            {
                return Err(AppError::invalid());
            }
        }
        if fields.is_empty() {
            return Err(AppError::invalid());
        }
        let rows = array(&data["rows"], 100)?;
        for row in rows {
            let row_object = row.as_object().ok_or_else(AppError::invalid)?;
            if row_object.len() != fields.len()
                || row_object.keys().any(|k| !fields.contains_key(k.as_str()))
            {
                return Err(AppError::invalid());
            }
            for (name, kind) in &fields {
                let value = &row[*name];
                if value.is_null() {
                    continue;
                }
                match *kind {
                    "number" => {
                        value
                            .as_f64()
                            .filter(|n| n.is_finite() && n.abs() <= 1e12)
                            .ok_or_else(AppError::invalid)?;
                    }
                    "timestamp" => {
                        instant(value)?;
                    }
                    _ => {
                        text(value, 2000)?;
                    }
                }
            }
        }
        if datasets.insert(data_id, Dataset { fields, rows }).is_some() {
            return Err(AppError::invalid());
        }
    }
    Check {
        datasets,
        nodes: HashSet::new(),
        content: 0,
    }
    .node(&spec["root"], 0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_executable_or_unbound_interface_content() {
        let safe = serde_json::json!({"schemaVersion":1,"kind":"components","root":{"id":"root","type":"text","text":"safe"},"datasets":[]});
        assert!(validate(&safe).is_ok());
        let mut bad = safe.clone();
        bad["root"]["html"] = Value::String("<script>evil()</script>".into());
        assert!(validate(&bad).is_err());
        let mut bad = safe;
        bad["root"] = serde_json::json!({"id":"metric","type":"metric","datasetId":"missing","field":"n","label":"x","format":"number"});
        assert!(validate(&bad).is_err());
    }
}
