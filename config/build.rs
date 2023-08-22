fn main() {

    let env = match option_env!("SLIPPI_ENV") {
        None => { "development" }
        Some(env) => { env }
    };
    println!("cargo:warning=Using Env {}", env);

    println!("cargo:rustc-cfg=feature=\"slippi_env_{}\"", env.to_lowercase());

}
