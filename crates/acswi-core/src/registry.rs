//! HKEY_CURRENT_USER keys saved as `registry` state items. Windows only; elsewhere the
//! items are refused.

pub use imp::{capture, clear, present, restore};

#[cfg(windows)]
mod imp {
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    use crate::error::{Error, Result};
    use crate::vault::{RegNode, RegValue};

    pub fn present(key: &str) -> Result<bool> {
        Ok(RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, KEY_READ).is_ok())
    }

    pub fn capture(key: &str) -> Result<Option<RegNode>> {
        match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, KEY_READ) {
            Ok(k) => Ok(Some(read_reg(&k)?)),
            Err(_) => Ok(None),
        }
    }

    pub fn clear(key: &str) -> Result<()> {
        match RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(key) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(format!("HKCU\\{key}"), e)),
        }
    }

    pub fn restore(key: &str, node: &RegNode) -> Result<()> {
        let (k, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(key)
            .map_err(|e| Error::io(format!("HKCU\\{key}"), e))?;
        write_reg(&k, node)
    }

    fn reg_type(n: u32) -> RegType {
        use RegType::*;
        match n {
            0 => REG_NONE,
            1 => REG_SZ,
            2 => REG_EXPAND_SZ,
            3 => REG_BINARY,
            4 => REG_DWORD,
            5 => REG_DWORD_BIG_ENDIAN,
            6 => REG_LINK,
            7 => REG_MULTI_SZ,
            8 => REG_RESOURCE_LIST,
            9 => REG_FULL_RESOURCE_DESCRIPTOR,
            10 => REG_RESOURCE_REQUIREMENTS_LIST,
            11 => REG_QWORD,
            _ => REG_BINARY,
        }
    }

    fn read_reg(key: &RegKey) -> Result<RegNode> {
        let mut node = RegNode::default();
        for v in key.enum_values() {
            let (name, value) = v.map_err(|e| Error::io("registry", e))?;
            node.values.push(RegValue { name, kind: value.vtype as u32, data: value.bytes });
        }
        for k in key.enum_keys() {
            let name = k.map_err(|e| Error::io("registry", e))?;
            let sub = key.open_subkey_with_flags(&name, KEY_READ).map_err(|e| Error::io(&name, e))?;
            node.keys.push((name, read_reg(&sub)?));
        }
        Ok(node)
    }

    fn write_reg(key: &RegKey, node: &RegNode) -> Result<()> {
        for v in &node.values {
            let value = winreg::RegValue { bytes: v.data.clone(), vtype: reg_type(v.kind) };
            key.set_raw_value(&v.name, &value).map_err(|e| Error::io(&v.name, e))?;
        }
        for (name, sub) in &node.keys {
            let (k, _) = key.create_subkey(name).map_err(|e| Error::io(name, e))?;
            write_reg(&k, sub)?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::error::{Code, Error, Result};
    use crate::vault::RegNode;

    fn unsupported() -> Error {
        Error::new(Code::Unsupported, "registry items only work on Windows")
    }

    pub fn present(_key: &str) -> Result<bool> {
        Err(unsupported())
    }

    pub fn capture(_key: &str) -> Result<Option<RegNode>> {
        Err(unsupported())
    }

    pub fn clear(_key: &str) -> Result<()> {
        Err(unsupported())
    }

    pub fn restore(_key: &str, _node: &RegNode) -> Result<()> {
        Err(unsupported())
    }
}
