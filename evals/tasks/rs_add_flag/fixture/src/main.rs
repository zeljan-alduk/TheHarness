fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let joined = args.join(" ");
    let out: String = joined.chars().rev().collect();
    println!("{out}");
}
