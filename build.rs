fn main() {
    println!("cargo:rerun-if-changed=src/database/migrations/merchant");
}
