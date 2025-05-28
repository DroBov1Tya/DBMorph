use mongodb::bson::{Bson, Document};
use std::collections::HashMap;

pub async fn flatten_bson(doc: &Document) -> String {
    let mut parts = Vec::new();

    for (k, v) in doc.iter() {
        let val = match v {
            Bson::String(s) => s.clone(),
            Bson::Int32(i) => i.to_string(),
            Bson::Int64(i) => i.to_string(),
            Bson::Double(f) => f.to_string(),
            Bson::Boolean(b) => b.to_string(),
            _ => format!("{:?}", v),
        };

        parts.push(format!("{}:{}", k, val));
    }

    parts.join(" ")
}
