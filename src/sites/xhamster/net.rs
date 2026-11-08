pub use crate::sites::net::encode;

pub const COOKIE: &str = "age_verified=1; platform=desktop; lang=en; x_csrf_token=1";

pub fn get(url: &str, referer: &str, xhr: bool) -> Result<String, String> {
    crate::sites::net::get(url, referer, COOKIE, xhr)
}
