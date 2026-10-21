use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::io::Cursor;

fn error(err: serde_yaml::Error) -> Json {
    json!({"display": err.to_string(), "debug": format!("{err:?}"),
        "location": err.location().map(|p| [p.index(), p.line(), p.column()])})
}

fn value(result: Result<serde_yaml::Value, serde_yaml::Error>) -> Json {
    match result {
        Ok(value) => {
            let mut merged = value.clone();
            let merge = match merged.apply_merge() {
                Ok(()) => {
                    json!({"value": format!("{merged:?}"), "yaml": serde_yaml::to_string(&merged).unwrap()})
                }
                Err(err) => error(err),
            };
            json!({"value": format!("{value:?}"), "yaml": serde_yaml::to_string(&value).unwrap(), "merge": merge})
        }
        Err(err) => error(err),
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum Tagged {
    Unit,
    Scalar(String),
    List(Vec<i64>),
    Record { name: String, n: i64 },
}

struct FailingReader;

impl std::io::Read for FailingReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("YAML fixture read failed"))
    }
}

pub fn behavior_report() -> Json {
    let cases = [
        "title: Café 日本語 🦀\nn: 42\nrows: [1, null, true]\n",
        "hex: 0xff\noctal: 0o777\nboolean: yes\ndate: 2024-01-01\n",
        "yes: yes\nno: no\non: on\noff: off\ntrue: TRUE\nfalse: False\n",
        "%YAML 1.1\n---\nyes: yes\nnumber: 012\n",
        "%YAML 1.2\n---\nyes: yes\nnumber: 012\n",
        "title: |\n  alpha\n  beta\n",
        "title: >-\n  alpha\n  beta\n",
        "a: &base {x: 1}\nb: {<<: *base, y: 2}\n",
        "a: &a {x: 1}\nb: &b {x: 2, y: 3}\nc: {<<: [*a, *b], z: 4}\n",
        "number: !!str 42\nvalue: !custom [1, 2]\n",
        "!!map {a: !!int '12', b: !!bool true}\n",
        "key: a\nkey: b\n",
        "x: .inf\ny: -.Inf\nz: .NaN\n",
        "min: -9223372036854775808\nmax: 9223372036854775807\nu64: 18446744073709551615\nlarge: 18446744073709551616\n",
        "float: 1.25e-3\nnegative_zero: -0.0\nseparators: 1_000\nsexagesimal: 1:20\n",
        "null: null\nempty:\ntilde: ~\nquoted: 'null'\n",
        "date: 2024-02-29\ntime: 2024-02-29T12:34:56Z\n",
        "? [a, b]\n: {1: 2}\n",
        "'é': \"\\u263a\\U0001f980\"\n",
        "a: [\n",
        "é: ok\nbad: [1, 2\n",
        "a:\n  b: 1\n c: 2\n",
        "a: *missing\n",
        "a: &loop [*loop]\n",
        "a: \"\\q\"\n",
        "a: !!int nope\n",
        "a: {<<: 3}\n",
        "null\n---\n42\n",
        "---\na: 1\n...\n",
        "",
        "# comment only\n",
        "[1, {a: 2}, null, false]\n",
        "a:\n  - b: 1\n    c: 2\n",
    ];
    let mut reports = Vec::new();
    for text in cases {
        let from_str = value(serde_yaml::from_str(text));
        let from_slice = value(serde_yaml::from_slice(text.as_bytes()));
        let from_reader = value(serde_yaml::from_reader(Cursor::new(text.as_bytes())));
        assert_eq!(from_str, from_slice, "slice: {text}");
        assert_eq!(from_str, from_reader, "reader: {text}");
        let json_value = match serde_yaml::from_str::<Json>(text) {
            Ok(value) => json!({"json": value, "yaml": serde_yaml::to_string(&value).unwrap()}),
            Err(err) => error(err),
        };
        let mut stream = Vec::new();
        for doc in serde_yaml::Deserializer::from_str(text) {
            let result = serde_yaml::Value::deserialize(doc);
            let failed = result.is_err();
            stream.push(value(result));
            // After a parse error the stream cannot advance to another document.
            if failed {
                break;
            }
        }
        reports.push(json!({"input": text, "yaml_value": from_str, "json_value": json_value, "stream": stream}));
    }
    let tagged = [
        Tagged::Unit,
        Tagged::Scalar("Café".into()),
        Tagged::List(vec![1, -2]),
        Tagged::Record {
            name: "日".into(),
            n: 3,
        },
    ];
    let tagged: Vec<_> = tagged.iter().map(|item| {
        let text = serde_yaml::to_string(item).unwrap();
        let value = serde_yaml::to_value(item).unwrap();
        assert_eq!(
            format!("{:?}", serde_yaml::from_value::<Tagged>(value).unwrap()),
            format!("{item:?}")
        );
        let mut serializer_output = Vec::new();
        item.serialize(&mut serde_yaml::Serializer::new(&mut serializer_output)).unwrap();
        assert_eq!(serializer_output, text.as_bytes());
        let mut writer = Vec::new();
        serde_yaml::to_writer(&mut writer, item).unwrap();
        assert_eq!(text.as_bytes(), writer);
        json!({"yaml": text, "roundtrip": format!("{:?}", serde_yaml::from_str::<Tagged>(&text).unwrap())})
    }).collect();
    let bad_utf8 = error(serde_yaml::from_slice::<serde_yaml::Value>(b"a: \xff").unwrap_err());
    let floats: Vec<_> = [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::MIN,
        f64::MAX,
    ]
    .into_iter()
    .map(|n| serde_yaml::to_string(&n).unwrap())
    .collect();
    let mut bibliographies = Vec::new();
    for input in [
        include_str!("../../../third-party/hayagriva/tests/data/basic.yml"),
        "book:\n  type: Book\n  title: Café\n  author: Doe, Jane\n  date: 2024-02-29\n  publisher: Example\n",
        "book:\n  title: Missing type\n",
        "book:\n  type: Book\n  date: invalid-date\n",
        "book: [\n",
    ] {
        let result = match hayagriva::io::from_yaml_str(input) {
            Ok(library) => json!({"serialized": hayagriva::io::to_yaml_str(&library).unwrap(), "debug": format!("{library:?}")}),
            Err(err) => error(err),
        };
        bibliographies.push(json!({"input": input, "result": result}));
    }
    let reader_error =
        error(serde_yaml::from_reader::<_, serde_yaml::Value>(FailingReader).unwrap_err());
    json!({"cases": reports, "tagged": tagged, "invalid_utf8": bad_utf8, "floats": floats, "bibliographies": bibliographies, "reader_error": reader_error})
}

#[test]
fn yaml_matches_frozen_serde_yaml_0_9_behavior() {
    let baseline: Json =
        serde_json::from_str(include_str!("fixtures/yaml-legacy-behavior.json")).unwrap();
    assert_eq!(behavior_report(), baseline);
}

#[test]
fn hayagriva_and_typst_share_yaml_error_and_value_types() {
    // This is the exact boundary that failed when only one consumer migrated.
    let err: serde_yaml::Error =
        hayagriva::io::from_yaml_str("bad:\n  title: Missing type\n").unwrap_err();
    assert_eq!(err.location().map(|p| (p.line(), p.column())), Some((2, 3)));
    assert!(err.to_string().contains("bad"));
    let _ = typst::loading::format_yaml_error(err);
    let valid = "book:\n  type: Book\n  title: Café\n  author: Doe, Jane\n  date: 2024-02-29\n  publisher: Example\n";
    let library = hayagriva::io::from_yaml_str(valid).unwrap();
    let encoded = hayagriva::io::to_yaml_str(&library).unwrap();
    let again = hayagriva::io::from_yaml_str(&encoded).unwrap();
    assert_eq!(format!("{library:?}"), format!("{again:?}"));
    assert_eq!(library.nth(0).unwrap().date().unwrap().year, 2024);
}

#[test]
fn maintained_backend_preserves_io_error_source() {
    let err = serde_yaml::from_reader::<_, serde_yaml::Value>(FailingReader).unwrap_err();
    let source = std::error::Error::source(&err).unwrap();
    assert!(source.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(source.to_string(), "YAML fixture read failed");
}
