use mongodb::bson::{Bson, Document};

pub async fn flatten_bson(doc: &Document) -> String {
    let mut parts = Vec::new();
    extract_bson_values(doc, &mut parts);
    parts.join(" ")
}

fn extract_bson_values(bson: &Document, parts: &mut Vec<String>) {
    for (_, v) in bson.iter() {
        match v {
            Bson::String(s) => parts.push(s.to_lowercase()),
            Bson::Int32(i) => parts.push(i.to_string()),
            Bson::Int64(i) => parts.push(i.to_string()),
            Bson::Double(f) => parts.push(f.to_string()),
            Bson::Boolean(b) => parts.push(b.to_string()),
            Bson::Array(arr) => {
                for item in arr {
                    match item {
                        Bson::Document(d) => extract_bson_values(d, parts),
                        Bson::String(s) => parts.push(s.to_lowercase()),
                        _ => parts.push(format!("{:?}", item)),
                    }
                }
            }
            Bson::Document(d) => extract_bson_values(d, parts),
            _ => parts.push(format!("{:?}", v)),
        }
    }
}
