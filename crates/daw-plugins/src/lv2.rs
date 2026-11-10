//! LV2 hosting (Linux only), filled in below.
use crate::PluginInfo;
use crate::native::{LoadRequest, NativeProcessor};
use anyhow::{Result, bail};
use std::path::Path;

pub(crate) fn load(_request: &LoadRequest<'_>) -> Result<Box<dyn NativeProcessor>> {
    bail!("LV2 is not implemented yet")
}

pub(crate) fn scan(_path: &Path) -> Result<Vec<PluginInfo>> {
    bail!("LV2 is not implemented yet")
}
