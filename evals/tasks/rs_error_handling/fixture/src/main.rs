fn main() {
    let args: Vec<String> = std::env::args().collect();
    let a: i64 = args[1].parse().unwrap();
    let b: i64 = args[2].parse().unwrap();
    println!("{}", a / b);
}
