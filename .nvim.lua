-- You must enable the exrc setting in neovim for this config file to be used.

local features = {
    "postcard_bindgen_core/generating",
    "postcard_bindgen/generating",
}

vim.g.rustaceanvim = {
    server = {
        settings = {
            ["rust-analyzer"] = {
                cargo = {
                    allTargets = false,
                    features = features,
                },
                check = { features = features, command = "clippy" },
                imports = {
                    preferNoStd = true,
                },
            },
        },
    },
}
