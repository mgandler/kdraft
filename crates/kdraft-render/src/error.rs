#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("font: {0}")]
    Font(String),
    #[error("PDF: {0}")]
    Pdf(String),
    #[error("raster: {0}")]
    Raster(String),
}
