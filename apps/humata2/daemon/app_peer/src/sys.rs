use std::io;
use std::os::fd::BorrowedFd;

/// What the app must be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    text: String,
}

impl Requirement {
    /// The Developer ID app of `team` with bundle identifier `bundle_id`. `None`
    /// when either is empty or not shaped like a team id (10 of `A-Z0-9`) and a
    /// reverse-DNS name, so nothing can be injected into the requirement text and
    /// an ad-hoc dev build (no team) never matches anything.
    pub fn app(team: &str, bundle_id: &str) -> Option<Self> {
        let team_ok = team.len() == 10
            && team
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        let bundle_ok = !bundle_id.is_empty()
            && bundle_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if !team_ok || !bundle_ok {
            return None;
        }
        // 1.2.840.113635.100.6.2.6 is Apple's "Developer ID Certification
        // Authority" intermediate and ...6.1.13 the "Developer ID Application"
        // leaf marker, so a development or distribution cert of the same team
        // doesn't pass.
        Some(Self {
            text: format!(
                "anchor apple generic \
                 and certificate 1[field.1.2.840.113635.100.6.2.6] \
                 and certificate leaf[field.1.2.840.113635.100.6.1.13] \
                 and certificate leaf[subject.OU] = \"{team}\" \
                 and identifier \"{bundle_id}\""
            ),
        })
    }
}

/// A process's `audit_token_t`: eight `u32`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditToken([u32; 8]);

impl AuditToken {
    /// The audit token of the process on the other end of a Unix socket
    /// (`LOCAL_PEERTOKEN`).
    pub fn of_peer(fd: BorrowedFd<'_>) -> io::Result<Self> {
        peer_token(fd)
    }

    /// The pid inside the token (`audit_token_to_pid`).
    pub fn pid(&self) -> i32 {
        self.0[5] as i32
    }
}

/// Why a process is not the Humata app.
#[derive(Debug, thiserror::Error)]
pub enum NotApp {
    #[error(
        "Security.framework has no running process for the socket peer (status {status}); \
         it probably exited, so reconnect from Humata"
    )]
    NoProcess { status: i32 },
    #[error(
        "the daemon's app requirement doesn't compile (status {status}); this is a bug in \
         the daemon build, reinstall Humata"
    )]
    BadRequirement { status: i32 },
    #[error(
        "the process on the socket isn't Humata signed by this team (status {status}); open \
         Humata from /Applications, or for a dev build run a dev daemon with `--admin`"
    )]
    Unmet { status: i32 },
    #[error(
        "couldn't read the signing information of the process on the socket (status {status}); \
         reopen Humata from /Applications"
    )]
    NoSigningInfo { status: i32 },
    #[error(
        "the process on the socket is Humata but wasn't signed with the hardened runtime; \
         install a release build of Humata from /Applications"
    )]
    NotHardened,
    #[error(
        "the process on the socket has the entitlement `{name}`, which lets others inject code \
         into it; install a release build of Humata from /Applications"
    )]
    Entitlement { name: &'static str },
    #[error("recognising the Humata app needs macOS; use a dev daemon with `--admin` here")]
    Unsupported,
}

/// Asks Security.framework whether the process behind `token` satisfies
/// `requirement`, has the hardened runtime and holds none of the entitlements
/// that allow code injection.
pub fn check(token: &AuditToken, requirement: &Requirement) -> Result<(), NotApp> {
    check_impl(token, requirement)
}

#[cfg(not(target_os = "macos"))]
fn peer_token(_fd: BorrowedFd<'_>) -> io::Result<AuditToken> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(target_os = "macos"))]
fn check_impl(_token: &AuditToken, _requirement: &Requirement) -> Result<(), NotApp> {
    Err(NotApp::Unsupported)
}

#[cfg(target_os = "macos")]
use mac::{check_impl, peer_token};

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::io;
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::ptr;

    use super::{AuditToken, NotApp, Requirement};

    type CFTypeRef = *const c_void;
    type OSStatus = i32;

    /// Only ever used by address, as the callbacks argument of `CFDictionaryCreate`.
    #[repr(C)]
    struct CFCallBacks {
        _private: [u8; 0],
    }

    // CoreFoundation/CFString.h: kCFStringEncodingUTF8.
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    // CoreFoundation/CFNumber.h: kCFNumberSInt64Type.
    const CF_NUMBER_SINT64_TYPE: isize = 4;
    // sys/un.h: SOL_LOCAL and LOCAL_PEERTOKEN.
    const SOL_LOCAL: i32 = 0;
    const LOCAL_PEERTOKEN: i32 = 0x006;
    // Security/CSCommon.h: kSecCSSigningInformation, kSecCSRequirementInformation
    // (the latter is what makes kSecCodeInfoEntitlementsDict appear).
    const SEC_CS_SIGNING_INFORMATION: u32 = 1 << 1;
    const SEC_CS_REQUIREMENT_INFORMATION: u32 = 1 << 2;
    // Security/CSCommon.h: kSecCodeSignatureRuntime, in kSecCodeInfoFlags.
    const SEC_CODE_SIGNATURE_RUNTIME: i64 = 0x1_0000;

    // Entitlements that let another process inject code into the app.
    const DANGEROUS_ENTITLEMENTS: [&str; 3] = [
        "com.apple.security.get-task-allow",
        "com.apple.security.cs.disable-library-validation",
        "com.apple.security.cs.allow-dyld-environment-variables",
    ];

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeDictionaryKeyCallBacks: CFCallBacks;
        static kCFTypeDictionaryValueCallBacks: CFCallBacks;
        fn CFRelease(cf: CFTypeRef);
        fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, length: isize) -> CFTypeRef;
        fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            is_external: u8,
        ) -> CFTypeRef;
        fn CFDictionaryCreate(
            alloc: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: isize,
            key_callbacks: *const CFCallBacks,
            value_callbacks: *const CFCallBacks,
        ) -> CFTypeRef;
        fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFNumberGetValue(number: CFTypeRef, kind: isize, out: *mut c_void) -> u8;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFDictionaryGetTypeID() -> usize;
    }

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        // Security/SecCode.h
        static kSecGuestAttributeAudit: CFTypeRef;
        fn SecCodeCopyGuestWithAttributes(
            host: CFTypeRef,
            attributes: CFTypeRef,
            flags: u32,
            guest: *mut CFTypeRef,
        ) -> OSStatus;
        fn SecCodeCheckValidity(code: CFTypeRef, flags: u32, requirement: CFTypeRef) -> OSStatus;
        // Security/SecRequirement.h
        fn SecRequirementCreateWithString(
            text: CFTypeRef,
            flags: u32,
            requirement: *mut CFTypeRef,
        ) -> OSStatus;
        // Security/SecStaticCode.h (a SecCodeRef is accepted as a SecStaticCodeRef)
        static kSecCodeInfoFlags: CFTypeRef;
        static kSecCodeInfoEntitlementsDict: CFTypeRef;
        fn SecCodeCopySigningInformation(
            code: CFTypeRef,
            flags: u32,
            information: *mut CFTypeRef,
        ) -> OSStatus;
    }

    // In libSystem, which every macOS program links.
    unsafe extern "C" {
        fn getsockopt(
            socket: i32,
            level: i32,
            name: i32,
            value: *mut c_void,
            len: *mut u32,
        ) -> i32;
    }

    /// Owns one retained CF object and releases it on drop.
    struct Cf(CFTypeRef);

    impl Cf {
        fn get(&self) -> CFTypeRef {
            self.0
        }
    }

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: self.0 is a non-null object we own one retain of (the
            // constructors below only wrap non-null "Create"/"Copy" results).
            unsafe { CFRelease(self.0) }
        }
    }

    fn wrap(raw: CFTypeRef) -> Option<Cf> {
        (!raw.is_null()).then_some(Cf(raw))
    }

    fn cf_string(text: &str) -> Option<Cf> {
        // SAFETY: the pointer and length describe `text`, which outlives the call;
        // CFStringCreateWithBytes copies the bytes.
        wrap(unsafe {
            CFStringCreateWithBytes(
                ptr::null(),
                text.as_ptr(),
                text.len() as isize,
                CF_STRING_ENCODING_UTF8,
                0,
            )
        })
    }

    pub(super) fn peer_token(fd: BorrowedFd<'_>) -> io::Result<AuditToken> {
        let mut token = [0u32; 8];
        let mut len = size_of_val(&token) as u32;
        // SAFETY: `token` is 32 writable bytes, `len` says so, and `fd` is open for
        // the duration of the borrow.
        let rc = unsafe {
            getsockopt(
                fd.as_raw_fd(),
                SOL_LOCAL,
                LOCAL_PEERTOKEN,
                token.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize != size_of_val(&token) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the kernel returned a short audit token for the socket peer",
            ));
        }
        Ok(AuditToken(token))
    }

    pub(super) fn check_impl(token: &AuditToken, requirement: &Requirement) -> Result<(), NotApp> {
        let code = guest(token)?;
        let req = compile(requirement)?;
        // SAFETY: `code` and `req` are live SecCode and SecRequirement objects.
        let status = unsafe { SecCodeCheckValidity(code.get(), 0, req.get()) };
        if status != 0 {
            return Err(NotApp::Unmet { status });
        }
        check_hardening(&code)
    }

    fn guest(token: &AuditToken) -> Result<Cf, NotApp> {
        let no_process = |status| NotApp::NoProcess { status };
        let mut bytes = [0u8; 32];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(token.0) {
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
        // SAFETY: `bytes` is 32 readable bytes; CFDataCreate copies them.
        let data = wrap(unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), 32) })
            .ok_or_else(|| no_process(-1))?;
        // SAFETY: reading an extern constant that Security.framework initialises
        // before any code runs.
        let key = unsafe { kSecGuestAttributeAudit };
        let keys = [key];
        let values = [data.get()];
        // SAFETY: one key and one value, both live CF objects; the callbacks are
        // the framework's own constants, used by address.
        let attrs = wrap(unsafe {
            CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
        })
        .ok_or_else(|| no_process(-1))?;
        let mut code: CFTypeRef = ptr::null();
        // SAFETY: `attrs` is a live dictionary and `code` a valid out pointer.
        let status = unsafe { SecCodeCopyGuestWithAttributes(ptr::null(), attrs.get(), 0, &mut code) };
        match wrap(code) {
            Some(code) if status == 0 => Ok(code),
            _ => Err(no_process(status)),
        }
    }

    fn compile(requirement: &Requirement) -> Result<Cf, NotApp> {
        let text = cf_string(&requirement.text).ok_or(NotApp::BadRequirement { status: -1 })?;
        let mut req: CFTypeRef = ptr::null();
        // SAFETY: `text` is a live CFString and `req` a valid out pointer.
        let status = unsafe { SecRequirementCreateWithString(text.get(), 0, &mut req) };
        match wrap(req) {
            Some(req) if status == 0 => Ok(req),
            _ => Err(NotApp::BadRequirement { status }),
        }
    }

    fn check_hardening(code: &Cf) -> Result<(), NotApp> {
        let mut info: CFTypeRef = ptr::null();
        // SAFETY: `code` is a live SecCode (valid as a static code too) and `info`
        // a valid out pointer.
        let status = unsafe {
            SecCodeCopySigningInformation(
                code.get(),
                SEC_CS_SIGNING_INFORMATION | SEC_CS_REQUIREMENT_INFORMATION,
                &mut info,
            )
        };
        let info = match wrap(info) {
            Some(info) if status == 0 => info,
            _ => return Err(NotApp::NoSigningInfo { status }),
        };
        // SAFETY: `info` is a live CFDictionary; the key is a framework constant.
        // The result is borrowed from `info` (Get rule), so it isn't released.
        let flags = unsafe { CFDictionaryGetValue(info.get(), kSecCodeInfoFlags) };
        if flags.is_null() || !is_number(flags) {
            return Err(NotApp::NotHardened);
        }
        let mut value: i64 = 0;
        // SAFETY: `flags` is a CFNumber and `value` is 8 writable bytes, matching
        // kCFNumberSInt64Type.
        let ok = unsafe { CFNumberGetValue(flags, CF_NUMBER_SINT64_TYPE, (&raw mut value).cast()) };
        if ok == 0 || value & SEC_CODE_SIGNATURE_RUNTIME == 0 {
            return Err(NotApp::NotHardened);
        }
        // SAFETY: as above; the entitlements dictionary is absent when there are none.
        let entitlements = unsafe { CFDictionaryGetValue(info.get(), kSecCodeInfoEntitlementsDict) };
        if entitlements.is_null() {
            return Ok(());
        }
        // SAFETY: `entitlements` is a non-null CF object.
        if unsafe { CFGetTypeID(entitlements) != CFDictionaryGetTypeID() } {
            return Ok(());
        }
        for name in DANGEROUS_ENTITLEMENTS {
            let Some(key) = cf_string(name) else { continue };
            // SAFETY: `entitlements` is a live dictionary and `key` a live CFString.
            let value = unsafe { CFDictionaryGetValue(entitlements, key.get()) };
            // SAFETY: a non-null `value` is a live CF object borrowed from the
            // dictionary; CFBooleanGetValue is only called on a CFBoolean.
            let set = !value.is_null()
                && unsafe {
                    CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value) != 0
                };
            if set {
                return Err(NotApp::Entitlement { name });
            }
        }
        Ok(())
    }

    fn is_number(value: CFTypeRef) -> bool {
        // SAFETY: `value` is a non-null CF object.
        unsafe { CFGetTypeID(value) == CFNumberGetTypeID() }
    }
}
