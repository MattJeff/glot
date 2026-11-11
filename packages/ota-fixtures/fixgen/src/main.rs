// Plural fixture for the OTA SDKs, straight from glot's validator rules (glot_core::plural, ICU4X CLDR).
fn main() {
    let plurals: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let locales: Vec<String> = plurals["supplemental"]["plurals-type-cardinal"]
        .as_object().unwrap().keys().filter(|k| *k != "root").cloned().collect();
    let mut nums: Vec<String> = (0..=30).chain(100..=111).chain([1000, 1000000]).map(|n| n.to_string()).collect();
    nums.extend(["0.0", "0.1", "0.5", "1.0", "1.1", "1.5", "1.25", "2.0", "2.5", "3.2", "10.1", "11.0", "21.0", "101.5"].map(String::from));
    let mut out = serde_json::Map::new();
    for l in locales {
        let cats: Vec<&str> = nums.iter().map(|n| glot_core::plural::category(&l, n, false).unwrap().as_str()).collect();
        // All "other" = not in ICU4X's compiled data (root fallback): glot's validator doesn't cover it either.
        let has_rules = plurals["supplemental"]["plurals-type-cardinal"][&l].as_object().unwrap().len() > 1;
        if has_rules && cats.iter().all(|c| *c == "other") {
            continue;
        }
        out.insert(l, serde_json::json!(cats));
    }
    println!("{}", serde_json::json!({"numbers": nums, "categories": out}));
}
