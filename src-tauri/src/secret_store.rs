//! Windows current-user DPAPI wrapper for secrets persisted by the host.

use base64::Engine as _;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{GetLastError, LocalFree},
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};

pub fn protect_current_user(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }

    #[cfg(windows)]
    {
        let mut input_bytes = plain.as_bytes().to_vec();
        let input = blob(&mut input_bytes)?;
        let mut output = empty_blob();
        let succeeded = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if succeeded == 0 {
            return Err(last_error("Windows DPAPI 加密失败"));
        }
        let bytes = copy_and_free(output);
        return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
    }

    #[cfg(not(windows))]
    {
        let _ = plain;
        Err("当前平台不支持 Windows DPAPI 密钥存储".into())
    }
}

pub fn unprotect_current_user(protected: &str) -> Result<String, String> {
    if protected.is_empty() {
        return Ok(String::new());
    }

    #[cfg(windows)]
    {
        let mut input_bytes = base64::engine::general_purpose::STANDARD
            .decode(protected)
            .map_err(|_| "保存的加密密钥格式无效".to_string())?;
        let input = blob(&mut input_bytes)?;
        let mut output = empty_blob();
        let succeeded = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if succeeded == 0 {
            return Err(last_error(
                "无法解密保存的密钥；它可能来自其他 Windows 用户或电脑",
            ));
        }
        let bytes = copy_and_free(output);
        return String::from_utf8(bytes).map_err(|_| "保存的加密密钥不是有效 UTF-8 文本".into());
    }

    #[cfg(not(windows))]
    {
        let _ = protected;
        Err("当前平台不支持 Windows DPAPI 密钥存储".into())
    }
}

#[cfg(windows)]
fn blob(bytes: &mut [u8]) -> Result<CRYPT_INTEGER_BLOB, String> {
    let length = u32::try_from(bytes.len()).map_err(|_| "密钥长度超过 Windows DPAPI 限制")?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: bytes.as_mut_ptr(),
    })
}

#[cfg(windows)]
fn empty_blob() -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    }
}

#[cfg(windows)]
fn copy_and_free(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if blob.pbData.is_null() {
        return Vec::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec() };
    unsafe {
        LocalFree(blob.pbData.cast());
    }
    bytes
}

#[cfg(windows)]
fn last_error(context: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("{context}（Windows 错误 {code}）")
}

#[cfg(all(test, windows))]
mod tests {
    use super::{protect_current_user, unprotect_current_user};

    #[test]
    fn protects_and_restores_a_secret_for_the_current_user() {
        let plain = "agile-tiles-dpapi-test";
        let protected = protect_current_user(plain).expect("protect secret");

        assert_ne!(protected, plain);
        assert_eq!(
            unprotect_current_user(&protected).expect("unprotect secret"),
            plain
        );
    }
}
