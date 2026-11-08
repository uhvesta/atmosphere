extension ModelLibraryClient {
    /// Apple's model, then each one downloaded, by name.
    public func runnable() async -> [RunnableModel] {
        RunnableModel.list(await installed())
    }
}
