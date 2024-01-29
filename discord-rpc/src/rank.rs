use serde::Deserialize;

#[derive(Deserialize)]
pub struct RankInfo {
    #[serde(alias = "rank")]
    pub name: String,
    #[serde(alias = "rating")]
    pub elo: f32
}

pub async fn get_rank_info(code: &str) -> Result<RankInfo, Box<dyn std::error::Error>> {
    let res = reqwest::get(format!("http://slprank.com/rank/{}?raw", code)).await?;
    Ok(res.json::<RankInfo>().await?)
}