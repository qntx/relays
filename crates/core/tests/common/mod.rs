pub(crate) const BASE: &str = r#"
[[collection]]
id = "featured"
name = { en = "Featured", zh-Hans = "精选" }
description = { en = "Hand-picked relays.", zh-Hans = "精心挑选的中继。" }
relays = ["wss://a.example/", "wss://b.example/x"]

[[collection]]
id = "global"
name = { en = "Global Feeds" }
relays = ["wss://b.example/x"]

[[relay]]
url = "wss://a.example/"
name = "Relay A"
description = { en = "The A relay.", zh-Hans = "A 中继。" }
languages = ["en"]
"#;
