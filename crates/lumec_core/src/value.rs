#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(transparent)]
pub struct Value(pub u64);

const TAG_POINTER: u64 = 0x0000;
const TAG_INT: u64 = 0x0001;
const TAG_BOOL: u64 = 0x0002;

impl Value {
    #[inline]
    pub fn int(i: i64) -> Self {
        let bits = (i as u64) << 2 | TAG_INT;
        Self(bits)
    }
    #[inline]
    pub fn bool(b: bool) -> Self {
        let v = if b { 1 } else { 0 };
        let bits = (v as u64) << 2 | TAG_BOOL;
        Self(bits)
    }
    #[inline]
    pub fn ptr(p: *mut u8) -> Self {
        let addr = p as u64;
        let bits = (addr << 2) | TAG_POINTER;
        Self(bits)
    }

    #[inline]
    pub fn tag(&self) -> u64 {
        self.0 & 0x3
    }
    #[inline]
    pub fn as_int(&self) -> i64 {
        (self.0 >> 2) as i64
    }
    #[inline]
    pub fn as_bool(&self) -> bool {
        (self.0 >> 2) & 1 != 0
    }
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        (self.0 >> 2) as *mut u8
    }

    pub fn is_int(&self) -> bool {
        self.tag() == TAG_INT
    }
    pub fn is_bool(&self) -> bool {
        self.tag() == TAG_BOOL
    }
    pub fn is_ptr(&self) -> bool {
        self.tag() == TAG_POINTER
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tag() {
            TAG_INT => write!(f, "{}", self.as_int()),
            TAG_BOOL => write!(f, "{}", self.as_bool()),
            TAG_POINTER => write!(f, "ptr({:p})", self.as_ptr()),
            _ => write!(f, "unknown value"),
        }
    }
}
