//! What every view needs besides the document while it builds its rows.

use crate::results::Shown;
use crate::validate::Problem;

pub struct Edit<'a> {
    /// The project's job names, for trigger pickers.
    pub jobs: &'a [String],
    /// The job being edited, and the latest run, whose results the rows show.
    pub job: &'a str,
    pub run: Option<Shown<'a>>,
    /// Why the document is invalid, shown on the job or task it is about.
    pub problem: Option<&'a Problem>,
}
