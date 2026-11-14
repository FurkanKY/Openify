//! Windows "Farklı kaydet" iletişim kutusu (yedek dosyası için).
//!
//! Modern `IFileSaveDialog`; bloklar, bu yüzden arayüz iş parçacığında DEĞİL,
//! kendi iş parçacığında çağrılır (`ui/tools.rs`). Kullanıcı vazgeçerse
//! `None`; iletişim kutusu hiç kurulamazsa (COM hatası) `fallback` yolu
//! döner, yedek yine de Belgeler\Openify yedekleri'ne yazılır.

use std::path::{Path, PathBuf};

use windows::{
    Win32::{
        Foundation::ERROR_CANCELLED,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
            FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileOpenDialog,
            FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
        },
    },
    core::{HSTRING, w},
};

fn show(title: &str, file_name: &str) -> windows::core::Result<PathBuf> {
    // SAFETY: COM çağrıları bu iş parçacığında, iletişim kutusu bitene dek.
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetFileName(&HSTRING::from(file_name))?;
        dialog.SetDefaultExtension(w!("json"))?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: w!("JSON"),
            pszSpec: w!("*.json"),
        }])?;
        let options = dialog.GetOptions()?;
        dialog
            .SetOptions(options | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM)?;
        dialog.Show(None)?;
        let item = dialog.GetResult()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(PathBuf::from(path?))
    }
}

fn show_open(title: &str) -> windows::core::Result<Vec<PathBuf>> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetFileTypes(&[
            COMDLG_FILTERSPEC {
                pszName: w!("Spotify history (JSON, ZIP)"),
                pszSpec: w!("*.json;*.zip"),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("JSON"),
                pszSpec: w!("*.json"),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("ZIP"),
                pszSpec: w!("*.zip"),
            },
        ])?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(
            options
                | FOS_ALLOWMULTISELECT
                | FOS_FILEMUSTEXIST
                | FOS_PATHMUSTEXIST
                | FOS_FORCEFILESYSTEM,
        )?;
        dialog.Show(None)?;
        let items = dialog.GetResults()?;
        let mut paths = Vec::new();
        for i in 0..items.GetCount()? {
            let item = items.GetItemAt(i)?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            paths.push(PathBuf::from(path?));
        }
        Ok(paths)
    }
}

/// Çoklu JSON / ZIP seçtirir; iptal ya da COM hatasında `None` döner.
pub fn open_files_dialog(title: &str) -> Option<Vec<PathBuf>> {
    let inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = show_open(title);
    if inited {
        unsafe { CoUninitialize() };
    }
    match result {
        Ok(paths) if !paths.is_empty() => Some(paths),
        Ok(_) => None,
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => None,
        Err(e) => {
            log::warn!("Aç iletişim kutusu açılamadı: {e}");
            None
        }
    }
}

/// Dosya seçtirir: seçilen yol; vazgeçildiyse `None`; iletişim kutusu
/// kurulamadıysa `fallback`.
pub fn save_file_dialog(title: &str, file_name: &str, fallback: &Path) -> Option<PathBuf> {
    // SAFETY: bu iş parçacığı kendi tek iş parçacıklı dairesini kurar ve bırakır.
    let inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = show(title, file_name);
    if inited {
        // SAFETY: yukarıdaki başarılı `CoInitializeEx` ile eşleşir.
        unsafe { CoUninitialize() };
    }
    match result {
        Ok(path) => Some(path),
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => None,
        Err(e) => {
            log::warn!("Kaydet iletişim kutusu açılamadı, varsayılan klasöre yazılıyor: {e}");
            Some(fallback.to_path_buf())
        }
    }
}
