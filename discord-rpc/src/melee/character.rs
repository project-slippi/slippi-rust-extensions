use std::fmt::Display;

use num_enum::TryFromPrimitive;

#[derive(Debug, Clone, Copy, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum MeleeCharacter {
    DrMario = 0x16,
	Mario = 0x08,
	Luigi = 0x07,
	Bowser = 0x05,
	Peach = 0x0C,
	Yoshi = 0x11,
	DonkeyKong = 0x01,
	CaptainFalcon = 0x00,
	Ganondorf = 0x19,
	Falco = 0x14,
	Fox = 0x02,
	Ness = 0x0B,
	IceClimbers = 0x0E,
	Kirby = 0x04,
	Samus = 0x10,
	Zelda = 0x12,
    Sheik = 0x13,
	Link = 0x06,
	YoungLink = 0x15,
	Pichu = 0x18,
	Pikachu = 0x0D,
	Jigglypuff = 0x0F,
	Mewtwo = 0x0A,
	MrGameAndWatch = 0x03,
	Marth = 0x09,
	Roy = 0x17,
	Hidden = 0xFF
}

impl MeleeCharacter {
	// useful when fetching from player card character address, however remains unused for now
	pub fn from_css(css_index: u8) -> Option<Self> {
		match css_index {
			0 => Some(Self::DrMario),
			1 => Some(Self::Mario),
			2 => Some(Self::Luigi),
			3 => Some(Self::Bowser),
			4 => Some(Self::Peach),
			5 => Some(Self::Yoshi),
			6 => Some(Self::DonkeyKong),
			7 => Some(Self::CaptainFalcon),
			8 => Some(Self::Ganondorf),
			9 => Some(Self::Falco),
			10 => Some(Self::Fox),
			11 => Some(Self::Ness),
			12 => Some(Self::IceClimbers),
			13 => Some(Self::Kirby),
			14 => Some(Self::Samus),
			15 => Some(Self::Zelda),
			16 => Some(Self::Link),
			17 => Some(Self::YoungLink),
			18 => Some(Self::Pichu),
			19 => Some(Self::Pikachu),
			20 => Some(Self::Jigglypuff),
			21 => Some(Self::Mewtwo),
			22 => Some(Self::MrGameAndWatch),
			23 => Some(Self::Marth),
			24 => Some(Self::Roy),
			_ => None
		}
	}
}

impl Display for MeleeCharacter {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match *self {
			Self::DrMario => write!(f, "Dr. Mario"),
			Self::Mario => write!(f, "Mario"),
			Self::Luigi => write!(f, "Luigi"),
			Self::Bowser => write!(f, "Bowser"),
			Self::Peach => write!(f, "Peach"),
			Self::Yoshi => write!(f, "Yoshi"),
			Self::DonkeyKong => write!(f, "Donkey Kong"),
			Self::CaptainFalcon => write!(f, "Captain Falcon"),
			Self::Ganondorf => write!(f, "Ganondorf"),
			Self::Falco => write!(f, "Falco"),
			Self::Fox => write!(f, "Fox"),
			Self::Ness => write!(f, "Ness"),
			Self::IceClimbers => write!(f, "Ice Climbers"),
			Self::Kirby => write!(f, "Kirby"),
			Self::Samus => write!(f, "Samus"),
			Self::Zelda => write!(f, "Zelda"),
			Self::Sheik => write!(f, "Sheik"),
			Self::Link => write!(f, "Link"),
			Self::YoungLink => write!(f, "Young Link"),
			Self::Pichu => write!(f, "Pichu"),
			Self::Pikachu => write!(f, "Pikachu"),
			Self::Jigglypuff => write!(f, "Jigglypuff"),
			Self::Mewtwo => write!(f, "Mewtwo"),
			Self::MrGameAndWatch => write!(f, "Mr. Game & Watch"),
			Self::Marth => write!(f, "Marth"),
			Self::Roy => write!(f, "Roy"),
			Self::Hidden => write!(f, "Hidden")
		}
	}
}

#[derive(Debug, PartialEq, Clone)]
pub struct OptionalMeleeCharacter(pub Option<MeleeCharacter>);
impl OptionalMeleeCharacter {
	pub fn as_discord_resource(&self) -> String {
		self.0.as_ref().and_then(|c|
			if *c == MeleeCharacter::Hidden { Some("transparent".into()) }
			else { Some(format!("char{}", (*c) as u8) ) }
		).unwrap_or("questionmark".into())
	}
}
impl Display for OptionalMeleeCharacter {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &(*self).0 {
			Some(v) => write!(f, "{}", v),
			_ => write!(f, "Unknown character")
		}
	}
}
