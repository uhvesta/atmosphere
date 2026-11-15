//! What plays a flow: anything that can do what a person does and say what
//! they would see.

use humata2_snapshot::RgbaImage;
use humata2_snapshot::layout::Layout;

use crate::flow::Action;

/// Plays a flow's actions against something real, and reports what it
/// shows. [`GpuiDriver`](crate::GpuiDriver) plays them in a headless GPUI
/// window; a driver for real daemons can play the same flows later.
pub trait Driver {
    /// Why an action or a snapshot failed.
    type Error: std::error::Error + Send + Sync + 'static;

    /// What the flow's artifact says played it, such as `gpui headless`.
    fn name(&self) -> &str;

    /// Does `action`, and returns once everything it started has settled.
    ///
    /// # Errors
    ///
    /// When the action can't be done, such as a control that isn't there.
    fn perform(&mut self, action: &Action) -> Result<(), Self::Error>;

    /// What is shown now, as a view snapshot (see
    /// [`humata2_snapshot::view`]).
    ///
    /// # Errors
    ///
    /// When what is shown can't be read.
    fn view(&mut self) -> Result<String, Self::Error>;

    /// How what is shown now is laid out, as a layout snapshot (see
    /// [`humata2_snapshot::layout`]), or `None` when this driver can't tell.
    ///
    /// # Errors
    ///
    /// When it can tell but reading the layout failed.
    fn layout(&mut self) -> Result<Option<Layout>, Self::Error>;

    /// What is shown now, drawn, or `None` when this driver can't draw
    /// pixels.
    ///
    /// # Errors
    ///
    /// When it can draw but drawing failed.
    fn image(&mut self) -> Result<Option<RgbaImage>, Self::Error>;
}
