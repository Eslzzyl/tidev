use anyhow::Result;

fn main() -> Result<()> {
    println!("tidev v{}", env!("CARGO_PKG_VERSION"));
    println!("（占位入口，尚未实现）");
    Ok(())
}
