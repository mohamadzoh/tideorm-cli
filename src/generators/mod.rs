//! Generator modules for TideORM CLI

pub mod factory;
pub mod migration;
pub mod model;
pub(crate) mod registry;
pub mod seeder;

use std::path::Path;

/// A generated file, checked and rendered but not written yet, so a command making several
/// files can check them all before it writes any.
pub struct Generated<'a> {
    path: String,
    content: String,
    /// Declares or registers the written file in its directory's `mod.rs`.
    register: Box<dyn FnOnce() -> Result<(), String> + 'a>,
}

impl<'a> Generated<'a> {
    pub(crate) fn new(
        path: String,
        content: String,
        register: impl FnOnce() -> Result<(), String> + 'a,
    ) -> Self {
        Self {
            path,
            content,
            register: Box::new(register),
        }
    }

    /// Write the file, creating its directory, register it, and return its path.
    pub fn write(self) -> Result<String, String> {
        if let Some(dir) = Path::new(&self.path).parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create directory '{}': {}", dir.display(), e))?;
        }
        std::fs::write(&self.path, self.content)
            .map_err(|e| format!("Failed to write {}: {}", self.path, e))?;
        (self.register)()?;
        Ok(self.path)
    }
}
