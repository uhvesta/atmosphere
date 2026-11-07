extension ModelLibraryClient {
    /// Apple's model, then each one downloaded, by name.
    package func runnable() async -> [RunnableModel] {
        RunnableModel.list(await installed())
    }
}
