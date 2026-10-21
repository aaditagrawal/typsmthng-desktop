#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct LegacyValues {
    text: String,
    characters: [char; 4],
    integers: (u128, i128),
    floats: (f32, f64),
    rows: std::collections::BTreeMap<String, Vec<i64>>,
    variants: Vec<LegacyVariant>,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum LegacyVariant {
    Empty,
    Pair(char, Option<u32>),
    Named { flag: bool, text: String },
}

fn legacy_values() -> LegacyValues {
    LegacyValues {
        text: "Café Ελληνικά مرحبا".into(),
        characters: ['\0', 'ß', 'λ', '😀'],
        integers: (u128::MAX, i128::MIN),
        floats: (-0.0, 1.25),
        rows: [("bounds".into(), vec![i64::MIN, -1, 0, i64::MAX])].into(),
        variants: vec![
            LegacyVariant::Empty,
            LegacyVariant::Pair('λ', None),
            LegacyVariant::Pair('😀', Some(0xdeadbeef)),
            LegacyVariant::Named { flag: true, text: "nested".into() },
        ],
    }
}
