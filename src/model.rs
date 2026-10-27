pub struct Model {
    pub title: &'static str,
    pub heading: &'static str,
    pub message: &'static str,
}

impl Model {
    pub const fn get() -> Self {
        Self {
            title: "Axum App",
            heading: "Hello from Axum",
            message: "A minimal MVC landing page.",
        }
    }
}
