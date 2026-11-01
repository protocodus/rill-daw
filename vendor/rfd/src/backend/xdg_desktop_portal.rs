use std::path::PathBuf;

use super::linux::zenity;
use crate::backend::DialogFutureType;
use crate::file_dialog::Filter;
use crate::message_dialog::MessageDialog;
use crate::{FileDialog, FileDialogError, FileHandle, MessageButtons, MessageDialogResult};

use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest, SaveFileRequest};
use ashpd::WindowIdentifier;

use log::error;
use pollster::block_on;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

fn to_window_identifier(
    window: Option<RawWindowHandle>,
    display: Option<RawDisplayHandle>,
) -> Option<WindowIdentifier> {
    window.map(|window| {
        block_on(Box::pin(async move {
            WindowIdentifier::from_raw_handle(&window, display.as_ref()).await
        }))
    })?
}

impl From<&Filter> for FileFilter {
    fn from(filter: &Filter) -> Self {
        let mut ashpd_filter = FileFilter::new(&filter.name);
        for file_extension in &filter.extensions {
            if file_extension == "*" || file_extension.is_empty() {
                ashpd_filter = ashpd_filter.glob("*");
            } else {
                ashpd_filter = ashpd_filter.glob(&format!("*.{file_extension}"));
            }
        }
        ashpd_filter
    }
}

fn is_cancelled(error: &ashpd::Error) -> bool {
    matches!(
        error,
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
            | ashpd::Error::Portal(ashpd::PortalError::Cancelled(_))
    )
}

/// Keep a rejected request distinct from a failed backend. A portal cancellation
/// must never open a second picker, and a failed fallback must reach the caller.
async fn file_dialog_result(
    dialog: FileDialog,
    save: bool,
) -> Result<Option<PathBuf>, FileDialogError> {
    let invalid_folder = |error| FileDialogError::new(format!("Invalid picker folder: {error}"));
    let request = if save {
        SaveFileRequest::default()
            .identifier(to_window_identifier(dialog.parent, dialog.parent_display))
            .title(dialog.title.as_deref())
            .current_name(dialog.file_name.as_deref())
            .filters(dialog.filters.iter().map(From::from))
            .current_folder::<&PathBuf>(&dialog.starting_directory)
            .map_err(invalid_folder)?
            .send()
            .await
    } else {
        OpenFileRequest::default()
            .identifier(to_window_identifier(dialog.parent, dialog.parent_display))
            .multiple(false)
            .title(dialog.title.as_deref())
            .filters(dialog.filters.iter().map(From::from))
            .current_folder::<&PathBuf>(&dialog.starting_directory)
            .map_err(invalid_folder)?
            .send()
            .await
    };
    match request {
        Ok(request) => match request.response() {
            Ok(response) => response
                .uris()
                .first()
                .ok_or_else(|| FileDialogError::new("Desktop file picker returned no selection"))
                .and_then(|uri| {
                    uri.to_file_path().map(Some).map_err(|_| {
                        FileDialogError::new("Desktop file picker returned a non-local file")
                    })
                }),
            Err(error) if is_cancelled(&error) => Ok(None),
            Err(error) => Err(FileDialogError::new(format!(
                "Desktop file picker failed: {error}"
            ))),
        },
        Err(error) if is_cancelled(&error) => Ok(None),
        Err(portal_error) => {
            let fallback = if save {
                zenity::save_file(&dialog).await
            } else {
                zenity::pick_file(&dialog).await
            };
            fallback.map_err(|error| {
                FileDialogError::new(format!(
                    "Desktop file picker unavailable: {portal_error}\nZenity fallback failed: {error}\nCheck that a desktop file portal is running or that zenity is installed and available on PATH, then retry the operation."
                ))
            })
        }
    }
}

//
// File Picker
//

use crate::backend::FilePickerDialogImpl;
impl FilePickerDialogImpl for FileDialog {
    fn pick_file(self) -> Option<PathBuf> {
        block_on(self.pick_file_async()).map(PathBuf::from)
    }

    fn pick_file_result(self) -> Result<Option<PathBuf>, FileDialogError> {
        block_on(file_dialog_result(self, false))
    }

    fn pick_files(self) -> Option<Vec<PathBuf>> {
        block_on(self.pick_files_async())
            .map(|vec_file_handle| vec_file_handle.iter().map(PathBuf::from).collect())
    }
}

use crate::backend::AsyncFilePickerDialogImpl;
impl AsyncFilePickerDialogImpl for FileDialog {
    fn pick_file_async(self) -> DialogFutureType<Option<FileHandle>> {
        Box::pin(async move {
            match file_dialog_result(self, false).await {
                Ok(selection) => selection.map(FileHandle::from),
                Err(error) => {
                    error!("Failed to pick file: {error}");
                    None
                }
            }
        })
    }

    fn pick_files_async(self) -> DialogFutureType<Option<Vec<FileHandle>>> {
        Box::pin(async move {
            let res = OpenFileRequest::default()
                .identifier(to_window_identifier(self.parent, self.parent_display))
                .multiple(true)
                .title(self.title.as_deref().or(None))
                .filters(self.filters.iter().map(From::from))
                .current_folder::<&PathBuf>(&self.starting_directory)
                .expect("File path should not be nul-terminated")
                .send()
                .await;

            if let Err(err) = res {
                error!("Failed to pick files: {err}");
                match zenity::pick_files(&self).await {
                    Ok(res) => Some(res.into_iter().map(FileHandle::from).collect::<Vec<_>>()),
                    Err(err) => {
                        error!("Failed to pick files with zenity: {err}");
                        None
                    }
                }
            } else {
                res.ok()
                    .and_then(|request| request.response().ok())
                    .map(|response| {
                        response
                            .uris()
                            .iter()
                            .filter_map(|uri| uri.to_file_path().ok())
                            .map(FileHandle::from)
                            .collect::<Vec<FileHandle>>()
                    })
            }
        })
    }
}

//
// Folder Picker
//

use crate::backend::FolderPickerDialogImpl;
impl FolderPickerDialogImpl for FileDialog {
    fn pick_folder(self) -> Option<PathBuf> {
        block_on(self.pick_folder_async()).map(PathBuf::from)
    }

    fn pick_folders(self) -> Option<Vec<PathBuf>> {
        block_on(self.pick_folders_async())
            .map(|vec_file_handle| vec_file_handle.iter().map(PathBuf::from).collect())
    }
}

use crate::backend::AsyncFolderPickerDialogImpl;
impl AsyncFolderPickerDialogImpl for FileDialog {
    fn pick_folder_async(self) -> DialogFutureType<Option<FileHandle>> {
        Box::pin(async move {
            let res = OpenFileRequest::default()
                .identifier(to_window_identifier(self.parent, self.parent_display))
                .multiple(false)
                .directory(true)
                .title(self.title.as_deref().or(None))
                .filters(self.filters.iter().map(From::from))
                .current_folder::<&PathBuf>(&self.starting_directory)
                .expect("File path should not be nul-terminated")
                .send()
                .await;

            if let Err(err) = res {
                error!("Failed to pick folder: {err}");
                match zenity::pick_folder(&self).await {
                    Ok(res) => res,
                    Err(err) => {
                        error!("Failed to pick folder with zenity: {err}");
                        return None;
                    }
                }
            } else {
                res.ok()
                    .and_then(|request| request.response().ok())
                    .and_then(|response| {
                        response
                            .uris()
                            .first()
                            .and_then(|uri| uri.to_file_path().ok())
                    })
            }
            .map(FileHandle::from)
        })
    }

    fn pick_folders_async(self) -> DialogFutureType<Option<Vec<FileHandle>>> {
        Box::pin(async move {
            let res = OpenFileRequest::default()
                .identifier(to_window_identifier(self.parent, self.parent_display))
                .multiple(true)
                .directory(true)
                .title(self.title.as_deref().or(None))
                .filters(self.filters.iter().map(From::from))
                .current_folder::<&PathBuf>(&self.starting_directory)
                .expect("File path should not be nul-terminated")
                .send()
                .await;

            if let Err(err) = res {
                error!("Failed to pick folders: {err}");
                match zenity::pick_folders(&self).await {
                    Ok(res) => Some(res.into_iter().map(FileHandle::from).collect::<Vec<_>>()),
                    Err(err) => {
                        error!("Failed to pick folders with zenity: {err}");
                        None
                    }
                }
            } else {
                res.ok()
                    .and_then(|request| request.response().ok())
                    .map(|response| {
                        response
                            .uris()
                            .iter()
                            .filter_map(|uri| uri.to_file_path().ok())
                            .map(FileHandle::from)
                            .collect::<Vec<FileHandle>>()
                    })
            }
        })
    }
}

//
// File Save
//

use crate::backend::FileSaveDialogImpl;
impl FileSaveDialogImpl for FileDialog {
    fn save_file(self) -> Option<PathBuf> {
        block_on(self.save_file_async()).map(PathBuf::from)
    }

    fn save_file_result(self) -> Result<Option<PathBuf>, FileDialogError> {
        block_on(file_dialog_result(self, true))
    }
}

use crate::backend::AsyncFileSaveDialogImpl;
impl AsyncFileSaveDialogImpl for FileDialog {
    fn save_file_async(self) -> DialogFutureType<Option<FileHandle>> {
        Box::pin(async move {
            match file_dialog_result(self, true).await {
                Ok(selection) => selection.map(FileHandle::from),
                Err(error) => {
                    error!("Failed to save file: {error}");
                    None
                }
            }
        })
    }
}

use crate::backend::MessageDialogImpl;
impl MessageDialogImpl for MessageDialog {
    fn show(self) -> MessageDialogResult {
        block_on(self.show_async())
    }
}

use crate::backend::AsyncMessageDialogImpl;
impl AsyncMessageDialogImpl for MessageDialog {
    fn show_async(self) -> DialogFutureType<MessageDialogResult> {
        Box::pin(async move {
            match &self.buttons {
                MessageButtons::Ok | MessageButtons::OkCustom(_) => {
                    let res = crate::backend::linux::zenity::message(
                        &self.level,
                        &self.buttons,
                        &self.title,
                        &self.description,
                    )
                    .await;

                    match res {
                        Ok(res) => res,
                        Err(err) => {
                            error!("Failed to open zenity dialog: {err}");
                            MessageDialogResult::Cancel
                        }
                    }
                }
                MessageButtons::OkCancel
                | MessageButtons::YesNo
                | MessageButtons::OkCancelCustom(..)
                | MessageButtons::YesNoCancel
                | MessageButtons::YesNoCancelCustom(..) => {
                    let res = crate::backend::linux::zenity::question(
                        &self.buttons,
                        &self.title,
                        &self.description,
                    )
                    .await;

                    match res {
                        Ok(res) => res,
                        Err(err) => {
                            error!("Failed to open zenity dialog: {err}");
                            MessageDialogResult::Cancel
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod result_tests {
    use super::*;

    #[test]
    fn only_explicit_portal_cancellation_is_a_cancel() {
        assert!(is_cancelled(&ashpd::Error::Response(
            ashpd::desktop::ResponseError::Cancelled,
        )));
        assert!(is_cancelled(&ashpd::Error::Portal(
            ashpd::PortalError::Cancelled("cancelled".into()),
        )));
        assert!(!is_cancelled(&ashpd::Error::Response(
            ashpd::desktop::ResponseError::Other,
        )));
        assert!(!is_cancelled(&ashpd::Error::NoResponse));
    }
}
