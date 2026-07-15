use core::error::Error;
use core::fmt;

/// Errors reported by fallible allocator operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocErr {
    /// Arithmetic overflow while computing a size or alignment.
    Overflow,
    /// The block source could not provide more memory.
    OutOfMemory,
    /// The requested alignment is not valid for this allocator.
    InvalidAlignment,
    /// A slot was freed while its bitmap bit already said it was free.
    DoubleFree,
    /// `free` received a pointer that does not belong to any pool block.
    InvalidPointer,
    /// `free` received a pointer inside a block but not on a slot boundary.
    MisalignedFree,
    /// A typed allocation requested a type too large or too aligned for a slot.
    InvalidSizeOrAlignement,
}

impl AllocErr {
    /// Panics with the human-readable allocator error.
    pub fn panic(self) -> ! {
        panic!("{}", self.to_string())
    }
}
impl fmt::Display for AllocErr {
    /// Formats the error message shown by panic-based APIs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllocErr::Overflow => {
                write!(f, "allocation size overflow")
            }
            AllocErr::OutOfMemory => {
                write!(f, "out of memory")
            }
            AllocErr::InvalidAlignment => {
                write!(f, "invalid memory alignment")
            }
            AllocErr::DoubleFree => {
                write!(f, "attempted to free memory more than once")
            }
            AllocErr::InvalidPointer => {
                write!(
                    f,
                    "free() called with a pointer that is not owned by this pool"
                )
            }
            AllocErr::MisalignedFree => {
                write!(f, "free() called with a misaligned pointer")
            }
            AllocErr::InvalidSizeOrAlignement => {
                write!(
                    f,
                    "object size or alignment exceeds the pool's slot size or alignment"
                )
            }
        }
    }
}

impl Error for AllocErr {}
