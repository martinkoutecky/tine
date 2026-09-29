fn main() {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(&input).unwrap();
    let result: Vec<_> = fixtures
        .iter()
        .map(|f| {
            tine_core::block_regions::parse(f["raw"].as_str().unwrap(), f["org"].as_bool().unwrap())
        })
        .collect();
    println!("{}", serde_json::to_string(&result).unwrap());
}
