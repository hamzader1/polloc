use core::error::Error;
use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocErr {
    Overflow,
    OutOfMemory,
    InvalidAlignment,
    DoubleFree,
    InvalidPointer,
    MisalignedFree,
    InvalidSizeOrAlignement,
}

impl AllocErr {
    pub fn panic(self) -> ! {
        panic!("{}", self.to_string())
    }
}
impl fmt::Display for AllocErr {
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
