use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocErr {
    Overflow,
    OutOfMemory,
    InvalidAlignment,
    DoubleFree,
}

impl AllocErr {
    fn panic(self) -> ! {
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
        }
    }
}

impl Error for AllocErr {}
