//! The achievements page: the game's RetroAchievements, earned ones first,
//! each with its badge, title, description and points; above them how many
//! are earned, and a bar. Until they come it says so (or why they could
//! not), and the badges show up once they are fetched.

use cosmic_text::Align;
use tiny_skia::Pixmap;

use super::{Page, Panel, TRACK_ALPHA, TRACK_THICKNESS, Target, fill};
use crate::achievements::{Achievement, Earned};
use crate::ui::text::{Fonts, TextBox};

/// An achievement's row, relative to the other rows.
const ROW_SCALE: f32 = 1.7;
/// How much the text of an achievement still to earn fades.
const LOCKED_FADE: f32 = 0.35;

/// The page, ready to draw.
pub(super) struct AchievementsPage {
    pub title: String,
    /// Shown while there is no list: "loading", or what went wrong.
    message: String,
    pub list: Vec<Achievement>,
    /// Each one's badge, once fetched.
    badges: Vec<Option<Pixmap>>,
    /// "12 of 40 · 120 of 400 points", and the share earned.
    summary: String,
    earned: f32,
}

impl Panel {
    /// Show the achievements page, saying `message` until they come.
    pub fn open_achievements(&mut self, title: String, message: String) {
        let page = AchievementsPage {
            title,
            message,
            list: Vec::new(),
            badges: Vec::new(),
            summary: String::new(),
            earned: 0.0,
        };
        self.show_page(Page::Achievements(page), 0);
    }

    /// The badges' size, in pixels.
    pub fn badge_size(&self) -> u32 {
        (self.achievement_height() * 0.8).round() as u32
    }

    /// The achievements came: `summary` says how many are earned.
    pub fn show_achievements(&mut self, list: Vec<Achievement>, summary: String, empty: String) {
        let Some(Page::Achievements(page)) = &mut self.page else {
            return;
        };
        let earned = list.iter().filter(|item| item.earned != Earned::No).count();
        page.earned = earned as f32 / list.len().max(1) as f32;
        page.summary = summary;
        page.message = empty;
        page.badges = vec![None; list.len()];
        page.list = list;
        self.fix_selection();
    }

    /// They could not come.
    pub fn achievements_failed(&mut self, message: String) {
        if let Some(Page::Achievements(page)) = &mut self.page {
            page.message = message;
        }
    }

    /// Their badges (in the list's order) came.
    pub fn show_badges(&mut self, badges: Vec<Option<Pixmap>>) {
        if let Some(Page::Achievements(page)) = &mut self.page
            && badges.len() == page.list.len()
        {
            page.badges = badges;
        }
    }

    /// An achievement's row, without its separator.
    pub(super) fn achievement_height(&self) -> f32 {
        (self.row_height() * ROW_SCALE).round()
    }

    /// The summary and its bar above the rows (0: none yet).
    pub(super) fn summary_band(&self) -> f32 {
        match &self.page {
            Some(Page::Achievements(page)) if !page.summary.is_empty() => {
                self.small.font.line_height + self.bar_thickness() + self.padding()
            }
            _ => 0.0,
        }
    }

    fn bar_thickness(&self) -> f32 {
        (self.row_height() * TRACK_THICKNESS).max(3.0).round()
    }

    /// The rows (or the message), then the summary over what scrolled
    /// under it.
    pub(super) fn draw_achievements(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        page: &AchievementsPage,
        padding: f32,
    ) {
        let width = self.width as f32;
        let color = self.text.style.color;
        if page.list.is_empty() {
            let (top, _) = self.list_area();
            let area = TextBox {
                x: padding,
                y: top + padding,
                width: width - 2.0 * padding,
                height: self.row_height(),
                align: Align::Center,
            };
            fonts.draw(pixmap, &self.small.font, &page.message, area, color);
            return;
        }

        let line = self.line();
        let height = self.achievement_height();
        let (rows, mut y) = self.shown_rows();
        for index in rows {
            let target = Target::Item(index);
            self.draw_row_background(pixmap, target, y, height);
            self.draw_achievement(pixmap, fonts, page, index, (padding, y));
            y += height;
            fill(pixmap, 0.0, y, width, line, self.text.style.separator_color);
            y += line;
        }

        // the summary, and the share earned as a bar
        let top = self.header_height();
        let band = self.summary_band();
        fill(pixmap, 0.0, top, width, band, self.background);
        let font = &self.small.font;
        let area = TextBox {
            x: padding,
            y: top + padding / 2.0,
            width: width - 2.0 * padding,
            height: font.line_height,
            align: Align::Center,
        };
        fonts.draw(pixmap, font, &page.summary, area, color);
        let thickness = self.bar_thickness();
        let bar_y = area.y + font.line_height;
        let bar_width = width - 2.0 * padding;
        let track = color.with_alpha(TRACK_ALPHA.min((color.0 & 0xff) as u8));
        fill(pixmap, padding, bar_y, bar_width, thickness, track);
        fill(
            pixmap,
            padding,
            bar_y,
            bar_width * page.earned,
            thickness,
            color,
        );
    }

    /// A row: the badge, the title over the description, the points.
    fn draw_achievement(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        page: &AchievementsPage,
        index: usize,
        at: (f32, f32),
    ) {
        let (padding, y) = at;
        let achievement = &page.list[index];
        let target = Target::Item(index);
        let mut color = self.row_color(target, true);
        if achievement.earned == Earned::No && target != self.selected {
            color = color.mix(self.background, LOCKED_FADE);
        }
        let height = self.achievement_height();
        let badge = self.badge_size() as f32;
        let badge_y = y + (height - badge) / 2.0;
        if let Some(Some(image)) = page.badges.get(index) {
            crate::ui::blit(
                pixmap,
                image,
                (padding.round() as i32, badge_y.round() as i32),
            );
        } else {
            fill(
                pixmap,
                padding,
                badge_y,
                badge,
                badge,
                color.with_alpha(TRACK_ALPHA),
            );
        }

        let small = &self.small.font;
        let right = self.width as f32 - padding;
        let points = achievement.points.to_string();
        let points_width = fonts.measure(&self.text.font, &points) + 1.0;
        let points_area = TextBox {
            x: right - points_width,
            y,
            width: points_width,
            height,
            align: Align::Right,
        };
        fonts.draw(pixmap, &self.text.font, &points, points_area, color);

        let x = padding + badge + padding * 0.5;
        let width = (right - points_width - padding * 0.5 - x).max(1.0);
        let lines = self.text.font.line_height + small.line_height;
        let top = y + (height - lines) / 2.0;
        let (title_font, title) = fonts.fit(&self.text.font, &achievement.title, width, 0.75);
        let title_area = TextBox {
            x,
            y: top,
            width,
            height: self.text.font.line_height,
            align: Align::Left,
        };
        fonts.draw(pixmap, &title_font, &title, title_area, color);
        let (description_font, description) =
            fonts.fit(small, &achievement.description, width, 0.8);
        let description_area = TextBox {
            x,
            y: top + self.text.font.line_height,
            width,
            height: small.line_height,
            align: Align::Left,
        };
        fonts.draw(
            pixmap,
            &description_font,
            &description,
            description_area,
            color,
        );
    }
}
