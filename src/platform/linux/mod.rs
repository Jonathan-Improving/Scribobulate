//! Linux-only plumbing that is not display-backend plumbing (that is `platform::x11`).
//!
//! | Module | Why only Linux needs it |
//! |---|---|
//! | [`process`] | The memory gates read this process's footprint from `/proc` and opt it out of the kernel's huge-page collapse, both Linux interfaces. |

pub(crate) mod process;
