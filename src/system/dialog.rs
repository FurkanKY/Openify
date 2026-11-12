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
        System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog, IFileSaveDialog,
            SIGDN_FILESYSPATH,
        },
    },
    core::{HSTRING, w},
};

fn show(title: &str, file_name: &str) -> windows::core::Result<PathBuf> {
    // SAFETY: COM çağrıları bu iş parçacığında, iletişim kutusu bitene dek.
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetFileName(&HSTRING::from(file_name))?;
        dialog.SetDefaultExtension(w!("json"))?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: w!("JSON"), pszSpec: w!("*.json") }])?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(options | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM)?;
        dialog.Show(None)?;
        let item = dialog.GetResult()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(PathBuf::from(path?))
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
