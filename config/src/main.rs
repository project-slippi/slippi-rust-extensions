use slippi_config::SlippiConfig;

pub fn main() {
    let configuration = SlippiConfig::get();
    println!("{:?}", configuration);
}