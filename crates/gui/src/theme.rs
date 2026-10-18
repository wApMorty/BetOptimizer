//! Couleurs et petits composants partagés par les onglets.

use edge_core::format;
use eframe::egui::{self, Color32, RichText, Ui};

/// Vert (gain, CLV positif), lisible en thème clair comme en thème sombre.
pub fn positive(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(110, 200, 140)
    } else {
        Color32::from_rgb(26, 127, 75)
    }
}

/// Rouge (perte, CLV négatif).
pub fn negative(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(240, 139, 131)
    } else {
        Color32::from_rgb(179, 38, 30)
    }
}

/// Ambre (avertissement).
pub fn warning(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(242, 196, 107)
    } else {
        Color32::from_rgb(138, 90, 0)
    }
}

/// Pourcentage signé coloré : vert au-dessus de zéro, rouge en dessous.
pub fn signed_percent(ui: &Ui, fraction: f64, digits: usize) -> RichText {
    let text = format!("{} %", format::signed(fraction * 100.0, digits));
    let color = if fraction > 0.0 {
        positive(ui)
    } else if fraction < 0.0 {
        negative(ui)
    } else {
        ui.visuals().text_color()
    };
    RichText::new(text).color(color)
}

/// Montant en euros signé et coloré.
pub fn signed_euros(ui: &Ui, amount: f64) -> RichText {
    let text = format!("{} €", format::signed(amount, 2));
    let color = if amount > 0.0 {
        positive(ui)
    } else if amount < 0.0 {
        negative(ui)
    } else {
        ui.visuals().text_color()
    };
    RichText::new(text).color(color)
}

/// Bandeau d'avertissement : les recommandations ne reposent sur aucune règle validée.
pub fn experimental_banner(ui: &mut Ui) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(
            RichText::new(
                "Expérimental. La probabilité est celle de Pinnacle sans marge, pas celle d'un modèle ; aucune règle n'est \
                 validée hors échantillon. Vérifie la cote sur le site de l'opérateur : sous la cote minimale, ne parie pas.",
            )
            .color(warning(ui)),
        );
    });
}
