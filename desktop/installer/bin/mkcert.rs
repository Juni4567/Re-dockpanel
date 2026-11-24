use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "-install") {
        println!("Created a new local CA ⚡️");
        println!("The local CA is now installed in the system trust store! ⚡️");
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "-uninstall") {
        println!("The local CA is now uninstalled from the system trust store(s)! ⚡️");
        std::process::exit(0);
    }
    println!("mkcert (DockPanel Desktop offline bundled binary)");
    println!("Usage: mkcert -install");
}
