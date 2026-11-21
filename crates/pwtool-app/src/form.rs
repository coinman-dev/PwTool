//! Form descriptions converted from the original `.dfm` files
//! (`assets/forms/*.json`, produced by `tools/dfm2json.py`).

use serde_json::Value;

/// One object of a form (control, menu item, dialog component…).
#[derive(Debug, Clone)]
pub struct FormObj {
    pub name: String,
    pub class: String,
    pub props: serde_json::Map<String, Value>,
    pub children: Vec<FormObj>,
}

impl FormObj {
    pub fn from_json(v: &Value) -> FormObj {
        FormObj {
            name: v["name"].as_str().unwrap_or("").to_string(),
            class: v["class"].as_str().unwrap_or("").to_string(),
            props: v["props"].as_object().cloned().unwrap_or_default(),
            children: v["children"].as_array().map(|a| a.iter().map(FormObj::from_json).collect()).unwrap_or_default(),
        }
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.props.get(key).and_then(|v| v.as_i64())
    }

    pub fn int_or(&self, key: &str, def: i64) -> i64 {
        self.int(key).unwrap_or(def)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.props.get(key).and_then(|v| v.as_str())
    }

    pub fn bool_or(&self, key: &str, def: bool) -> bool {
        self.props.get(key).and_then(|v| v.as_bool()).unwrap_or(def)
    }

    /// Identifier value (`alTop`, `csDropDownList`, a component name…).
    pub fn ident(&self, key: &str) -> Option<&str> {
        self.props.get(key).and_then(|v| v.get("id")).and_then(|v| v.as_str())
    }

    /// Set value (`[akLeft, akTop]`, `[fsBold]`).
    pub fn set(&self, key: &str) -> Option<Vec<String>> {
        self.props
            .get(key)
            .and_then(|v| v.get("set"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
    }

    /// String list (`Items.Strings = (…)`, `Lines.Strings`).
    pub fn strings(&self, key: &str) -> Vec<String> {
        self.props
            .get(key)
            .and_then(|v| v.get("list"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    /// Items of a collection property (`Columns`, `Panels`, `Images`…).
    pub fn collection(&self, key: &str) -> Vec<serde_json::Map<String, Value>> {
        self.props
            .get(key)
            .and_then(|v| v.get("items"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|i| i.get("props").and_then(|p| p.as_object()).cloned()).collect())
            .unwrap_or_default()
    }

    pub fn find(&self, name: &str) -> Option<&FormObj> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }

    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a FormObj)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }
}

/// Embedded form descriptions (`assets/forms/*.json`): name, whether
/// compressed, JSON (see build.rs).
static FORMS: &[(&str, bool, &[u8])] = include!(concat!(env!("OUT_DIR"), "/forms.rs"));

/// Load one of the embedded forms by name (e.g. `"Main"`).
pub fn load_form(name: &str) -> FormObj {
    try_load_form(name).unwrap_or_else(|| panic!("unknown form {name}"))
}

/// Load an embedded form, `None` if there is no such form.
pub fn try_load_form(name: &str) -> Option<FormObj> {
    let (_, z, data) = FORMS.iter().find(|(n, _, _)| *n == name)?;
    let bytes = crate::resources::unpack(*z, data);
    let json = std::str::from_utf8(&bytes).expect("form JSON");
    let v: Value = serde_json::from_str(json).expect("form JSON");
    Some(FormObj::from_json(&v))
}
