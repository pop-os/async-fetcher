fn main() {
    #[cfg(not(any(feature = "isahc", feature = "reqwest")))]
    compile_error!("At least one feature (isahc or reqwest) must be enabled");

    #[cfg(all(feature = "isahc", feature = "reqwest"))]
    compile_error!("Multiple backends enabled, use one of isahc or reqwest");
}
