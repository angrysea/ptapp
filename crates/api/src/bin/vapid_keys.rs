//! Prints a new VAPID private key (base64url) for Web Push. Used once by
//! scripts/deploy.sh, which stores it as an SSM SecureString; the Lambda
//! derives the public key from it.

fn main() {
    let (private, _public) = api::push::generate_keys();
    println!("{private}");
}
