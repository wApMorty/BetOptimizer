//! Mouvement : durées, courbes et apparition en cascade.
//!
//! Règles : un mouvement doit **dire quelque chose** (« ceci vient d'apparaître », « ceci a changé ») ; 150 à 300 ms avec
//! décélération (ça démarre vite et se pose doucement) ; jamais plus de 1,4 s au total par écran ; et tout peut être coupé
//! (« mouvement réduit ») : sans mouvement, chaque progression vaut 1 dès la première image.
//! Les valeurs de durée sont des choix du projet, inspirés des recommandations de Material Design 3 (non vérifiées à la source).

use eframe::egui::{self, Ui};

/// Durée d'une apparition de carte.
pub const REVEAL_SECS: f32 = 0.28;
/// Décalage entre deux cartes consécutives.
pub const STAGGER_SECS: f32 = 0.05;
/// Distance de glissement d'une apparition.
pub const REVEAL_RISE: f32 = 10.0;
/// Durée d'un défilement de chiffre.
pub const COUNT_SECS: f32 = 0.7;
/// Durée d'un tracé de courbe.
pub const DRAW_SECS: f32 = 0.9;
/// Durée après laquelle un écran est considéré comme posé (plus aucune image à forcer).
pub const SETTLE_SECS: f64 = 1.8;

/// L'horloge de mouvement d'un écran.
#[derive(Debug, Clone, Copy)]
pub struct Motion {
    /// Le mouvement est-il activé ?
    pub enabled: bool,
    /// Instant (secondes d'egui) où l'écran affiché est apparu.
    pub started: f64,
    /// Instant courant.
    pub now: f64,
}

/// Décélération cubique : `1 − (1 − t)³`.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

impl Motion {
    /// Progression (0 à 1, avec décélération) d'un mouvement de durée `duration` qui commence `delay` secondes après l'apparition
    /// de l'écran. Vaut 1 si le mouvement est coupé.
    pub fn progress(&self, delay: f32, duration: f32) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        let elapsed = (self.now - self.started) as f32 - delay;
        ease_out(elapsed / duration)
    }

    /// Progression linéaire (pour les mouvements qui suivent une grandeur, comme un tracé).
    pub fn linear(&self, delay: f32, duration: f32) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        (((self.now - self.started) as f32 - delay) / duration).clamp(0.0, 1.0)
    }

    /// Reste-t-il un mouvement en cours ? Si oui, il faut redessiner à la prochaine image.
    pub fn is_animating(&self) -> bool {
        self.enabled && self.now - self.started < SETTLE_SECS
    }

    /// Demande une nouvelle image tant qu'un mouvement est en cours.
    pub fn keep_alive(&self, ctx: &egui::Context) {
        if self.is_animating() {
            ctx.request_repaint();
        }
    }
}

/// Apparition en cascade : l'élément `index` d'un écran monte de [`REVEAL_RISE`] points en devenant opaque.
///
/// La hauteur totale ne change pas (l'espace retiré au-dessus est rendu en dessous), donc rien ne saute pendant le mouvement.
pub fn reveal<R>(
    ui: &mut Ui,
    motion: &Motion,
    index: usize,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let progress = motion.progress(STAGGER_SECS * index as f32, REVEAL_SECS);
    let offset = REVEAL_RISE * (1.0 - progress);
    ui.add_space(offset);
    let result = ui
        .scope(|ui| {
            ui.set_opacity(progress);
            add_contents(ui)
        })
        .inner;
    ui.add_space(REVEAL_RISE - offset);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_easing_starts_fast_ends_slow_and_stays_in_range() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(
            ease_out(0.5) > 0.8,
            "décélération : plus de 80 % du chemin à mi-parcours"
        );
        assert_eq!(ease_out(-3.0), 0.0);
        assert_eq!(ease_out(9.0), 1.0);
    }

    fn motion(enabled: bool, now: f64) -> Motion {
        Motion {
            enabled,
            started: 10.0,
            now,
        }
    }

    #[test]
    fn a_delayed_item_waits_for_its_turn_then_arrives() {
        let m = motion(true, 10.1);

        assert_eq!(m.progress(0.2, 0.28), 0.0, "pas encore commencé");
        assert!(m.progress(0.0, 0.28) > 0.0);
        assert_eq!(motion(true, 11.0).progress(0.2, 0.28), 1.0, "arrivé");
    }

    #[test]
    fn with_reduced_motion_everything_is_immediately_in_place() {
        let m = motion(false, 10.0);

        assert_eq!(m.progress(0.5, 0.28), 1.0);
        assert_eq!(m.linear(0.5, 0.9), 1.0);
        assert!(!m.is_animating());
    }

    #[test]
    fn a_screen_stops_requesting_frames_once_settled() {
        assert!(motion(true, 10.5).is_animating());
        assert!(!motion(true, 10.0 + SETTLE_SECS + 0.1).is_animating());
    }
}
