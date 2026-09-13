use std::collections::HashSet;

use super::enumerate;
use super::id::display_slug;

use crate::error::Result;

pub struct DdcDevice {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
}

pub fn list_ddc_monitors() -> Result<Vec<DdcDevice>> {
    let probed = enumerate::probe_ddc_monitors()?;
    let mut devices = Vec::new();
    let mut seen_ids = HashSet::new();

    for (index, monitor) in probed.into_iter().enumerate() {
        let base_id = format!("ddc:{}", display_slug(&monitor.display_name));
        let id = if index == 0 && !seen_ids.contains(&base_id) {
            base_id
        } else {
            format!("{base_id}:{index}")
        };

        if !seen_ids.insert(id.clone()) {
            continue;
        }

        devices.push(DdcDevice {
            id,
            name: monitor.description.clone(),
            display_name: monitor.display_name,
            description: monitor.description,
        });
    }

    Ok(devices)
}
