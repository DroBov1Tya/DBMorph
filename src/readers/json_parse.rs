// use futures::stream::{self, Stream};
// use serde_json::Value;
// use std::collections::HashMap;
// use tokio::fs::File;
// use tokio::io::{AsyncBufReadExt, BufReader};

// fn flatten_json(value: &Value, prefix: String, out: &mut HashMap<String, String>) {
//     match value {
//         Value::Object(map) => {
//             for (k, v) in map {
//                 let new_prefix = if prefix.is_empty() {
//                     k.to_string()
//                 } else {
//                     format!("{}.{}", prefix, k)
//                 };
//                 flatten_json(v, new_prefix, out);
//             }
//         }
//         Value::Array(arr) => {
//             for (i, v) in arr.iter().enumerate() {
//                 let new_prefix = format!("{}.{}", prefix, i);
//                 flatten_json(v, new_prefix, out);
//             }
//         }
//         Value::Null => {
//             out.insert(prefix, "null".to_string());
//         }
//         other => {
//             out.insert(prefix, other.to_string());
//         }
//     }
// }

// pub async fn read_json_lines_flat<P: AsRef<std::path::Path>>(
//     path: P,
// ) -> impl Stream<Item = Result<HashMap<String, String>, String>> {
//     let file = match File::open(path).await {
//         Ok(f) => f,
//         Err(e) => {
//             return stream::once(async { Err(format!("Failed to open file: {}", e)) }).boxed()
//         }
//     };

//     let reader = BufReader::new(file);
//     let lines = reader.lines();

//     stream::unfold(lines, |mut lines| async {
//         loop {
//             match lines.next_line().await {
//                 Ok(Some(line)) => match serde_json::from_str::<Value>(&line) {
//                     Ok(json) => {
//                         let mut map = HashMap::new();
//                         flatten_json(&json, "".to_string(), &mut map);
//                         return Some((Ok(map), lines));
//                     }
//                     Err(e) => {
//                         return Some((Err(format!("JSON parse error: {}", e)), lines));
//                     }
//                 },
//                 Ok(None) => return None,
//                 Err(e) => return Some((Err(format!("I/O error: {}", e)), lines)),
//             }
//         }
//     })
//     .boxed()
// }
