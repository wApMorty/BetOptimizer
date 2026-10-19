//! Photographier l'application : `edge-gui --demo --screenshot DOSSIER`.
//!
//! egui sait capturer **sa propre fenêtre** (`ViewportCommand::Screenshot`) : rien de l'écran de l'utilisateur n'est lu. L'image
//! est écrite en PNG par un encodeur minimal (blocs non compressés) pour ne pas ajouter de dépendance.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use eframe::egui::ColorImage;

use crate::pages::Tab;

/// CRC-32 (polynôme 0xEDB88320), tel que l'exige PNG.
fn crc32(chunks: &[&[u8]]) -> u32 {
    let table: Vec<u32> = (0..256u32)
        .map(|n| {
            (0..8).fold(n, |c, _| {
                if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                }
            })
        })
        .collect();
    let mut crc = 0xFFFF_FFFFu32;
    for chunk in chunks {
        for byte in *chunk {
            crc = table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
        }
    }
    !crc
}

/// Somme de contrôle Adler-32 d'un flux zlib.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// Encode des pixels RGBA 8 bits en PNG (flux zlib en blocs « stockés », donc sans compression).
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in rgba.chunks(width as usize * 4) {
        raw.push(0); // filtre « aucun »
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        zlib.push(u8::from(last));
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, sans interlacement
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    png
}

/// Écrit une capture d'egui dans un fichier PNG.
pub fn save(image: &ColorImage, path: &Path) -> Result<()> {
    let [width, height] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    let png = encode_png(width as u32, height as u32, &rgba);
    std::fs::write(path, png).with_context(|| format!("écriture de {}", path.display()))
}

/// Une vue à photographier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shot {
    /// Nom du fichier (sans extension).
    pub name: &'static str,
    /// Écran affiché.
    pub tab: Tab,
    /// Sous-vue de l'écran de performance (0 bankroll, 1 CLV, 2 calibration).
    pub segment: usize,
    /// Ouvrir le tiroir de saisie hors modèle ?
    pub drawer: bool,
}

/// Les vues photographiées, dans l'ordre.
pub const SHOTS: [Shot; 8] = [
    Shot {
        name: "01-aujourd-hui",
        tab: Tab::Today,
        segment: 0,
        drawer: false,
    },
    Shot {
        name: "02-opportunites",
        tab: Tab::Opportunities,
        segment: 0,
        drawer: false,
    },
    Shot {
        name: "03-paris",
        tab: Tab::Bets,
        segment: 0,
        drawer: false,
    },
    Shot {
        name: "04-paris-tiroir",
        tab: Tab::Bets,
        segment: 0,
        drawer: true,
    },
    Shot {
        name: "05-performance-bankroll",
        tab: Tab::Performance,
        segment: 0,
        drawer: false,
    },
    Shot {
        name: "06-performance-clv",
        tab: Tab::Performance,
        segment: 1,
        drawer: false,
    },
    Shot {
        name: "07-performance-calibration",
        tab: Tab::Performance,
        segment: 2,
        drawer: false,
    },
    Shot {
        name: "08-rapports",
        tab: Tab::Reports,
        segment: 0,
        drawer: false,
    },
];

/// Le déroulé d'une série de captures.
#[derive(Debug)]
pub struct Plan {
    /// Dossier de sortie.
    pub dir: PathBuf,
    /// Vue en cours.
    pub index: usize,
    /// La capture de la vue en cours a-t-elle été demandée ?
    pub requested: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksums_match_their_published_test_vectors() {
        assert_eq!(crc32(&[b"123456789"]), 0xCBF4_3926);
        assert_eq!(
            crc32(&[b"1234", b"56789"]),
            0xCBF4_3926,
            "le CRC se calcule sur des morceaux"
        );
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    /// Relit un PNG produit par `encode_png` : vérifie les CRC, lit l'en-tête et reconstitue les pixels.
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let mut at = 8;
        let (mut width, mut height) = (0, 0);
        let mut zlib = Vec::new();
        while at < png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let data = &png[at + 8..at + 8 + length];
            let stored =
                u32::from_be_bytes(png[at + 8 + length..at + 12 + length].try_into().unwrap());
            assert_eq!(
                crc32(&[kind, data]),
                stored,
                "CRC du bloc {}",
                String::from_utf8_lossy(kind)
            );
            match kind {
                b"IHDR" => {
                    width = u32::from_be_bytes(data[0..4].try_into().unwrap());
                    height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                    assert_eq!(&data[8..], &[8, 6, 0, 0, 0]);
                }
                b"IDAT" => zlib.extend_from_slice(data),
                _ => {}
            }
            at += 12 + length;
        }
        // Flux zlib en blocs stockés : 2 octets d'en-tête, puis [drapeau, LEN, NLEN, données]…, puis Adler-32.
        assert_eq!(&zlib[..2], &[0x78, 0x01]);
        let mut raw = Vec::new();
        let mut at = 2;
        loop {
            let last = zlib[at] == 1;
            let len = u16::from_le_bytes([zlib[at + 1], zlib[at + 2]]) as usize;
            let nlen = u16::from_le_bytes([zlib[at + 3], zlib[at + 4]]);
            assert_eq!(nlen, !(len as u16));
            raw.extend_from_slice(&zlib[at + 5..at + 5 + len]);
            at += 5 + len;
            if last {
                break;
            }
        }
        assert_eq!(
            u32::from_be_bytes(zlib[at..at + 4].try_into().unwrap()),
            adler32(&raw)
        );
        let pixels = raw
            .chunks(width as usize * 4 + 1)
            .flat_map(|row| {
                assert_eq!(row[0], 0, "filtre aucun");
                row[1..].to_vec()
            })
            .collect();
        (width, height, pixels)
    }

    #[test]
    fn a_small_image_survives_a_round_trip() {
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 11) as u8).collect();

        let (width, height, pixels) = decode(&encode_png(3, 2, &rgba));

        assert_eq!((width, height), (3, 2));
        assert_eq!(pixels, rgba);
    }

    #[test]
    fn an_image_larger_than_one_stored_block_is_split_and_still_decodes() {
        // 200 × 100 × 4 = 80 000 octets + 100 octets de filtre : deux blocs de 65 535 octets au plus.
        let rgba: Vec<u8> = (0..200 * 100 * 4).map(|i| (i % 251) as u8).collect();

        let (width, height, pixels) = decode(&encode_png(200, 100, &rgba));

        assert_eq!((width, height), (200, 100));
        assert_eq!(pixels, rgba);
    }
}
