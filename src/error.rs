use libc::c_int;

#[derive(Debug, Clone, Copy)]
pub enum Error {
    NoEntry,
    ParentNotFound,
    ChildNotFound,
    ApiCallFailed,
}

impl From<Error> for c_int {
    fn from(e: Error) -> Self {
        match e {
            Error::NoEntry => libc::ENOENT,
            Error::ParentNotFound => libc::ENOENT,
            Error::ChildNotFound => libc::ENOENT,
            Error::ApiCallFailed => libc::EIO,
        }
    }
}
