//! Linux-only plumbing that is not display-backend plumbing (that is `platform::x11`).
//!
//! | Module | Why only Linux needs it |
//! |---|---|
//! | [`automount`] | An `autofs` map mounts a remote host when a name inside it is looked up, so the image and link gates ask the mount table which directories are maps. |
//! | [`process`] | The memory gates read this process's footprint from `/proc` and opt it out of the kernel's huge-page collapse, both Linux interfaces. |

pub(crate) mod automount;
pub(crate) mod process;
