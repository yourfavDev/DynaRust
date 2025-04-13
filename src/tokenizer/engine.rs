// use regex::Regex;
//
// fn tokenize(sql: &str) -> Vec<String> {
//     // The regex pattern with verbosity enabled.
//     // We use non-capturing groups for alternatives and capture one of these groups.
//     let token_pattern = r#"(?x)
//         ( '[^']*' )      |   # single-quoted strings
//         ( "[^"]*" )      |   # double-quoted strings
//         ( \b\w+\b )      |   # word tokens (identifiers or keywords)
//         ( [(),;] )           # punctuation tokens
//     "#;
//
//     let re = Regex::new(token_pattern).unwrap();
//     let mut tokens = Vec::new();
//
//     // Iterate over each regex match and capture the non-null group.
//     for cap in re.captures_iter(sql) {
//         for i in 1..=4 {
//             if let Some(m) = cap.get(i) {
//                 tokens.push(m.as_str().to_string());
//                 break;
//             }
//         }
//     }
//     tokens
// }
//
// /// Extracts the table name from a SELECT SQL query.
// /// It tokenizes the SQL statement and then looks for the FROM keyword.
// /// The token immediately after FROM is assumed to be the table name.
// ///
// /// Returns `Some(table_name)` if a table is found, `None` otherwise.
// pub fn get_table_name(sql: &str) -> Option<String> {
//     let tokens = tokenize(sql);
//
//     // Verify that the first token is SELECT (case-insensitive).
//     if tokens.first()?.to_uppercase() != "SELECT" {
//         return None;
//     }
//
//     // Look for the FROM keyword and return the token immediately after it.
//     for (i, token) in tokens.iter().enumerate() {
//         if token.to_uppercase() == "FROM" && i + 1 < tokens.len() {
//             return Some(tokens[i + 1].clone());
//         }
//     }
//     None
// }