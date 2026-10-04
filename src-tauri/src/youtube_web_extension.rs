//! Linux WebProcess request hook, compiled as a separate, embedded shared object.
//! WebKitWebPage::send-request applies header mutations before sending, including
//! redirects (WebKit WebKitWebPage.cpp). UI-process resource signals do not.

#![cfg_attr(not(test), no_std)]

use core::ffi::{c_char, c_void, CStr};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    // Match panic=abort across the native callback boundary.
    unsafe { abort() }
}

const REFERER: &str = concat!(env!("TINE_YOUTUBE_REFERER"), "\0");

extern "C" {
    #[cfg(not(test))]
    fn abort() -> !;
    fn g_signal_connect_data(
        instance: *mut c_void,
        signal: *const c_char,
        callback: unsafe extern "C" fn(),
        data: *mut c_void,
        destroy: *mut c_void,
        flags: u32,
    ) -> usize;
    fn g_uri_parse(uri: *const c_char, flags: u32, error: *mut c_void) -> *mut c_void;
    fn g_uri_get_host(uri: *mut c_void) -> *const c_char;
    fn g_uri_get_scheme(uri: *mut c_void) -> *const c_char;
    fn g_uri_unref(uri: *mut c_void);
    fn webkit_uri_request_get_uri(request: *mut c_void) -> *const c_char;
    fn webkit_uri_request_get_http_headers(request: *mut c_void) -> *mut c_void;
    fn soup_message_headers_replace(headers: *mut c_void, name: *const c_char, value: *const c_char);
    fn soup_message_headers_get_one(headers: *mut c_void, name: *const c_char) -> *const c_char;
    fn soup_message_headers_remove(headers: *mut c_void, name: *const c_char);
}

fn youtube_host(host: &[u8]) -> bool {
    [b"youtube.com".as_slice(), b"youtube-nocookie.com".as_slice()]
        .iter()
        .any(|domain| {
            host.eq_ignore_ascii_case(domain)
                || (host.len() > domain.len()
                    && host[host.len() - domain.len() - 1] == b'.'
                    && host[host.len() - domain.len()..].eq_ignore_ascii_case(domain))
        })
}

unsafe extern "C" fn send_request(
    _page: *mut c_void,
    request: *mut c_void,
    redirect: *mut c_void,
    _data: *mut c_void,
) -> i32 {
    let uri = g_uri_parse(webkit_uri_request_get_uri(request), 0, core::ptr::null_mut());
    if uri.is_null() {
        return 0;
    }
    let host = g_uri_get_host(uri);
    let scheme = g_uri_get_scheme(uri);
    let youtube = !host.is_null()
        && !scheme.is_null()
        && CStr::from_ptr(scheme).to_bytes().eq_ignore_ascii_case(b"https")
        && youtube_host(CStr::from_ptr(host).to_bytes());
    let headers = webkit_uri_request_get_http_headers(request);
    if !headers.is_null() {
        if youtube {
            soup_message_headers_replace(
                headers,
                b"Referer\0".as_ptr().cast(),
                REFERER.as_ptr().cast(),
            );
        } else if !redirect.is_null() {
            // A redirect can carry the header injected into its prior request.
            // Do not forward our identity to a destination outside YouTube.
            let referer = soup_message_headers_get_one(headers, b"Referer\0".as_ptr().cast());
            if !referer.is_null() && CStr::from_ptr(referer).to_bytes_with_nul() == REFERER.as_bytes() {
                soup_message_headers_remove(headers, b"Referer\0".as_ptr().cast());
            }
        }
    }
    g_uri_unref(uri);
    0 // Do not cancel the request or suppress another extension's handler.
}

unsafe extern "C" fn page_created(
    _extension: *mut c_void,
    page: *mut c_void,
    _data: *mut c_void,
) {
    connect(page, b"send-request\0", send_request as *const ());
}

unsafe fn connect(instance: *mut c_void, signal: &[u8], callback: *const ()) {
    // GLib calls these callbacks with the documented signal's C ABI/signature.
    g_signal_connect_data(
        instance,
        signal.as_ptr().cast(),
        core::mem::transmute::<*const (), unsafe extern "C" fn()>(callback),
        core::ptr::null_mut(),
        core::ptr::null_mut(),
        0,
    );
}

#[no_mangle]
pub unsafe extern "C" fn webkit_web_extension_initialize(extension: *mut c_void) {
    connect(extension, b"page-created\0", page_created as *const ());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[link(name = "webkit2gtk-4.1")]
    #[link(name = "soup-3.0")]
    #[link(name = "gobject-2.0")]
    #[link(name = "glib-2.0")]
    extern "C" {
        fn webkit_uri_request_new(uri: *const c_char) -> *mut c_void;
        fn webkit_uri_request_set_uri(request: *mut c_void, uri: *const c_char);
        fn g_object_unref(object: *mut c_void);
    }

    #[test]
    fn redirected_identity_does_not_escape_youtube() {
        let youtube = CString::new("https://www.youtube.com/embed/JI-AyLv68Xs").unwrap();
        let other = CString::new("https://vimeo.com/123").unwrap();
        unsafe {
            let request = webkit_uri_request_new(youtube.as_ptr());
            send_request(core::ptr::null_mut(), request, core::ptr::null_mut(), core::ptr::null_mut());
            let headers = webkit_uri_request_get_http_headers(request);
            assert!(!soup_message_headers_get_one(headers, b"Referer\0".as_ptr().cast()).is_null());
            webkit_uri_request_set_uri(request, other.as_ptr());
            // Only the non-null redirect indication is used, never dereferenced.
            send_request(core::ptr::null_mut(), request, request, core::ptr::null_mut());
            assert!(soup_message_headers_get_one(headers, b"Referer\0".as_ptr().cast()).is_null());
            g_object_unref(request);
        }
    }

    #[test]
    fn native_request_hook_changes_only_youtube_referer() {
        for (url, youtube) in [
            ("https://www.youtube.com/embed/JI-AyLv68Xs?enablejsapi=1", true),
            ("https://www.youtube.com/youtubei/v1/player", true),
            ("https://WWW.YouTube.COM/iframe_api", true),
            ("https://www.youtube-nocookie.com/embed/JI-AyLv68Xs", true),
            ("http://www.youtube.com/embed/JI-AyLv68Xs", false),
            ("https://youtube.com.evil.test/embed/abc", false),
            ("https://www.youtube.com@evil.test/", false),
            ("https://vimeo.com/123", false),
            ("https://page.tine.tinebeta/", false),
        ] {
            let uri = CString::new(url).unwrap();
            unsafe {
                let request = webkit_uri_request_new(uri.as_ptr());
                let headers = webkit_uri_request_get_http_headers(request);
                assert!(!headers.is_null());
                soup_message_headers_replace(headers, b"Referer\0".as_ptr().cast(), b"https://existing.test/\0".as_ptr().cast());
                soup_message_headers_replace(headers, b"Cookie\0".as_ptr().cast(), b"test=preserved\0".as_ptr().cast());
                assert_eq!(send_request(core::ptr::null_mut(), request, core::ptr::null_mut(), core::ptr::null_mut()), 0);
                let actual = CStr::from_ptr(soup_message_headers_get_one(headers, b"Referer\0".as_ptr().cast()));
                let expected = if youtube { REFERER.trim_end_matches('\0') } else { "https://existing.test/" };
                assert_eq!(actual.to_str().unwrap(), expected, "{url}");
                assert_eq!(CStr::from_ptr(soup_message_headers_get_one(headers, b"Cookie\0".as_ptr().cast())).to_bytes(), b"test=preserved");
                g_object_unref(request);
            }
        }
    }

    #[test]
    fn identity_is_scoped_to_youtube_hosts() {
        for host in ["youtube.com", "www.youtube.com", "WWW.YouTube.COM", "youtube-nocookie.com", "www.youtube-nocookie.com"] {
            assert!(youtube_host(host.as_bytes()), "{host}");
        }
        for host in ["youtube.com.evil.test", "notyoutube.com", "youtube-nocookie.com.evil.test", "youtu.be", "vimeo.com", "page.tine.tinebeta", ""] {
            assert!(!youtube_host(host.as_bytes()), "{host}");
        }
    }
}
