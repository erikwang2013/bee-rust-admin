#[test]
fn probe() {
    for (k, v) in std::env::vars() {
        if k.contains("CARGO") {
            println!("{k} = {v}");
        }
    }
}
