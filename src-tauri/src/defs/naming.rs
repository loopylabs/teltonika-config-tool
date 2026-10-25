//! Model names for the device list (docs/spec/10-device-presentation.md §3).

use serde::Serialize;

use super::{compose, Library};

/// The model name of a hardware code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelName {
    /// The hardware table's FmType, alias-resolved for the name only (04 §3.1).
    pub name: Option<String>,
    /// False when no loaded definitions know the code: the UI shows the raw code with a hint.
    pub known: bool,
}

/// A provisional model taken from a Bluetooth name (our own design, spec 10 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelHint {
    pub model: String,
    /// The one hardware code with this FmType, when there is exactly one.
    pub hw_code: Option<String>,
}

impl Library {
    pub fn model_name(&self, hw_code: &str) -> ModelName {
        let name = self.find(hw_code).and_then(|(d, fam)| {
            let row = compose::resolve_row(&d.families[&fam], hw_code)?;
            Some(row.model_name.clone()).filter(|n| !n.is_empty())
        });
        ModelName { known: name.is_some(), name }
    }

    /// FmType (case-insensitive, exact) to hardware code. A hint exists only when some loaded
    /// build lists that FmType; the code only when the newest such build has exactly one.
    pub fn hint_for_model(&self, fm_type: &str) -> Option<ModelHint> {
        for (b, _) in &self.builds {
            let mut codes: Vec<&str> = Vec::new();
            let mut model = None;
            for f in b.families.values() {
                for r in f.hardware.iter().filter(|r| r.model_name.eq_ignore_ascii_case(fm_type)) {
                    model.get_or_insert(r.model_name.clone());
                    if !codes.iter().any(|c| c.eq_ignore_ascii_case(&r.hw_code)) {
                        codes.push(&r.hw_code);
                    }
                }
            }
            if let Some(model) = model {
                return Some(ModelHint { model, hw_code: (codes.len() == 1).then(|| codes[0].to_string()) });
            }
        }
        None
    }
}
