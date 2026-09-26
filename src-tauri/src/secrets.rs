use crate::error::{AppError, AppResult};
use std::fs;
use std::path::Path;

const SECRET_FILE_BYTES_MAX: u64 = 64 << 10;

#[cfg(windows)] // tigerstyle-ignore: TS035
mod dpapi {
    use crate::error::{AppError, AppResult};
    use std::ptr;
    use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPTPROTECT_UI_FORBIDDEN,
        CRYPT_INTEGER_BLOB,
        CryptProtectData,
        CryptUnprotectData,
    };

    fn blob_input(bytes: &[u8]) -> AppResult<CRYPT_INTEGER_BLOB> {
        let length = u32::try_from(bytes.len())
            .map_err(|_| AppError::Config("the secret is larger than DPAPI accepts".into()))?;

        Ok(CRYPT_INTEGER_BLOB { cbData: length, pbData: bytes.as_ptr().cast_mut() })
    }

    fn blob_output_take(blob: &CRYPT_INTEGER_BLOB, call: &str) -> AppResult<Vec<u8>> {
        if blob.pbData.is_null() {
            return Err(AppError::Config(format!("{call} returned no data")));
        }

        let bytes = unsafe {
            std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec()
        };

        let freed = unsafe { LocalFree(blob.pbData.cast()) };

        debug_assert!(freed.is_null());
        debug_assert_eq!(bytes.len(), blob.cbData as usize);

        Ok(bytes)
    }

    fn error_last(call: &str) -> AppError {
        let code = unsafe { GetLastError() };

        AppError::Config(format!("{call} failed (code {code})"))
    }

    pub(crate) fn protect(plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let blob_plain = blob_input(plaintext)?;

        let mut blob_cipher = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };

        let succeeded = unsafe {
            CryptProtectData(
                &blob_plain,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut blob_cipher,
            )
        };

        if succeeded == 0 {
            return Err(error_last("CryptProtectData"));
        }

        blob_output_take(&blob_cipher, "CryptProtectData")
    }

    pub(crate) fn unprotect(ciphertext: &[u8]) -> AppResult<Vec<u8>> {
        let blob_cipher = blob_input(ciphertext)?;

        let mut blob_plain = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };

        let succeeded = unsafe {
            CryptUnprotectData(
                &blob_cipher,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut blob_plain,
            )
        };

        if succeeded == 0 {
            return Err(error_last("CryptUnprotectData"));
        }

        blob_output_take(&blob_plain, "CryptUnprotectData")
    }
}

pub(crate) fn protected_write(path: &Path, plaintext: &str) -> AppResult<()> {
    debug_assert!(!plaintext.is_empty());
    debug_assert!(path.is_absolute());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(windows)] // tigerstyle-ignore: TS035
    {
        let ciphertext = dpapi::protect(plaintext.as_bytes())?;

        debug_assert_ne!(ciphertext.as_slice(), plaintext.as_bytes());

        crate::workspace::atomic_write(path, &ciphertext)?;
    }

    #[cfg(not(windows))] // tigerstyle-ignore: TS035
    {
        tracing::warn!(
            target: "scribe_lib::secrets",
            "writing secret as plaintext to {} (DPAPI only available on Windows)",
            path.display()
        );

        crate::workspace::atomic_write(path, plaintext.as_bytes())?;
    }

    debug_assert!(path.exists());

    Ok(())
}

pub(crate) fn protected_read(path: &Path) -> AppResult<Option<String>> {
    debug_assert!(path.is_absolute());

    if !path.exists() {
        return Ok(None);
    }

    let size = fs::metadata(path)?.len();

    if size > SECRET_FILE_BYTES_MAX {
        return Err(AppError::Config(format!(
            "{} is {size} bytes, over the {SECRET_FILE_BYTES_MAX} bytes a secret store can hold",
            path.display()
        )));
    }

    debug_assert!(size <= SECRET_FILE_BYTES_MAX);

    #[cfg(windows)] // tigerstyle-ignore: TS035
    {
        let ciphertext = fs::read(path)?;
        let plaintext_bytes = dpapi::unprotect(&ciphertext)?;

        let plaintext = String::from_utf8(plaintext_bytes)
            .map_err(|error| AppError::Config(format!("decrypted bytes are not utf-8: {error}")))?;

        Ok(Some(plaintext))
    }

    #[cfg(not(windows))] // tigerstyle-ignore: TS035
    {
        Ok(Some(fs::read_to_string(path)?))
    }
}

pub(crate) fn protected_delete(path: &Path) -> AppResult<()> {
    debug_assert!(path.is_absolute());

    if path.exists() {
        fs::remove_file(path)?;
    }

    debug_assert!(!path.exists());

    Ok(())
}
