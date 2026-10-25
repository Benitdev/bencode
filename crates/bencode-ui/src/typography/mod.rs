mod fit;
pub mod format;
mod inline;
pub mod keys;
mod math;
mod text;

pub use fit::Ellipsis;
pub(crate) use fit::LEADING;
pub use inline::{Highlight, Kbd, KbdCombo};
pub use math::{Latex, MathError};
pub use text::{Caption, Heading, Label, Overline, Paragraph, Subtitle, Title, literal, tabular};
