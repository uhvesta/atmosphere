import Foundation

/// The LaTeX a model writes for maths, as text with the symbols it stands
/// for: `\frac{a}{b}` as a/b, `x^2` as x², `\alpha` as α. Only what reads
/// on one line; what it doesn't know shows without its backslash.
enum Math {
    static func text(_ latex: String) -> String {
        var reader = Reader(chars: Array(latex))
        return reader.read(until: nil).replacingOccurrences(of: "  ", with: " ")
            .trimmingCharacters(in: .whitespaces)
    }

    /// Inline maths in `text`, between `$`s or `\(` and `\)`, as its
    /// symbols, escaped so Markdown leaves them be. Code spans are left as
    /// they are. A `$` opens maths only when text follows it straight away
    /// and closes it only when text comes straight before and no digit
    /// after, so prices such as $5 and $10 stay as they are.
    static func inline(_ text: String) -> String {
        let chars = Array(text)
        var out = ""
        var index = 0
        while index < chars.count {
            let char = chars[index]
            if char == "`" {
                let run = chars[index...].prefix { $0 == "`" }.count
                var end = index + run
                while end < chars.count {
                    if chars[end] == "`" {
                        let closing = chars[end...].prefix { $0 == "`" }.count
                        if closing == run { break }
                        end += closing
                    } else {
                        end += 1
                    }
                }
                let stop = min(end + run, chars.count)
                out += String(chars[index..<stop])
                index = stop
            } else if char == "\\", index + 1 < chars.count, chars[index + 1] == "(",
                let close = find(["\\", ")"], in: chars, from: index + 2)
            {
                out += escaped(Math.text(String(chars[(index + 2)..<close])))
                index = close + 2
            } else if char == "$", let close = dollarClose(chars, from: index) {
                out += escaped(Math.text(String(chars[(index + 1)..<close])))
                index = close + 1
            } else {
                out.append(char)
                index += 1
            }
        }
        return out
    }

    /// Where the `$` that closes the one at `start` is, or nil when it
    /// doesn't open maths.
    private static func dollarClose(_ chars: [Character], from start: Int) -> Int? {
        let first = start + 1
        guard first < chars.count, !chars[first].isWhitespace, chars[first] != "$",
            start == 0 || chars[start - 1] != "\\"
        else { return nil }
        var index = first
        while index < chars.count, chars[index] != "\n" {
            if chars[index] == "\\" {
                index += 2
                continue
            }
            if chars[index] == "$" {
                let next = index + 1
                guard !chars[index - 1].isWhitespace, next >= chars.count || !chars[next].isNumber else { return nil }
                return index
            }
            index += 1
        }
        return nil
    }

    private static func find(_ pattern: [Character], in chars: [Character], from start: Int) -> Int? {
        var index = start
        while index + pattern.count <= chars.count {
            if Array(chars[index..<(index + pattern.count)]) == pattern { return index }
            index += 1
        }
        return nil
    }

    private static func escaped(_ text: String) -> String {
        var out = ""
        for char in text {
            if "\\`*_[]<>#".contains(char) { out.append("\\") }
            out.append(char)
        }
        return out
    }

    /// The opening of a block of maths on a line of its own, `$$` or `\[`,
    /// and its closing.
    static func blockDelimiters(_ line: Substring) -> (open: String, close: String)? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("$$") { return ("$$", "$$") }
        if trimmed.hasPrefix("\\[") { return ("\\[", "\\]") }
        return nil
    }

    private struct Reader {
        let chars: [Character]
        var index = 0

        /// Reads up to `stop`, past it, or to the end.
        mutating func read(until stop: Character?) -> String {
            var out = ""
            while index < chars.count {
                let char = chars[index]
                if char == stop {
                    index += 1
                    return out
                }
                index += 1
                switch char {
                case "\\": out += command()
                case "{": out += read(until: "}")
                case "^": out += script(Math.superscripts, mark: "^")
                case "_": out += script(Math.subscripts, mark: "_")
                case "&": out += " "
                case "~": out += " "
                default: out.append(char)
                }
            }
            return out
        }

        /// One argument: a group in braces, a command, or a character.
        mutating func argument() -> String {
            while index < chars.count, chars[index] == " " { index += 1 }
            guard index < chars.count else { return "" }
            let char = chars[index]
            index += 1
            switch char {
            case "{": return read(until: "}")
            case "\\": return command()
            default: return String(char)
            }
        }

        /// A superscript or subscript, in the small characters when each
        /// has one, or after its mark otherwise.
        mutating func script(_ table: [Character: Character], mark: String) -> String {
            let text = argument()
            let small = text.compactMap { table[$0] }
            if small.count == text.count { return String(small) }
            return text.count == 1 ? mark + text : "\(mark)(\(text))"
        }

        mutating func command() -> String {
            guard index < chars.count else { return "" }
            let first = chars[index]
            guard first.isLetter else {
                index += 1
                switch first {
                case ",", ";", ":", " ", "!": return " "
                case "\\": return " "
                default: return String(first)
                }
            }
            let name = String(chars[index...].prefix { $0.isLetter })
            index += name.count
            switch name {
            case "frac", "dfrac", "tfrac":
                let top = argument()
                let bottom = argument()
                return "\(Self.grouped(top))/\(Self.grouped(bottom))"
            case "sqrt":
                let root = argument()
                return "√" + Self.grouped(root)
            case "text", "mathrm", "mathbf", "mathit", "operatorname", "textbf", "textit", "mathbb", "mathcal",
                "boldsymbol":
                let text = argument()
                return name == "mathbb" ? String(text.map { Math.doubleStruck[$0] ?? $0 }) : text
            case "left", "right", "big", "Big", "bigg", "Bigg", "displaystyle", "limits":
                return ""
            case "quad", "qquad":
                return "  "
            default:
                if let symbol = Math.symbols[name] { return symbol }
                return name
            }
        }

        /// `text` in brackets when it's more than one term.
        static func grouped(_ text: String) -> String {
            let plain = text.allSatisfy { $0.isLetter || $0.isNumber || $0 == "." }
            return plain || text.count == 1 ? text : "(\(text))"
        }
    }

    static let symbols: [String: String] = [
        "times": "×", "cdot": "·", "div": "÷", "pm": "±", "mp": "∓", "leq": "≤", "le": "≤", "geq": "≥",
        "ge": "≥", "neq": "≠", "ne": "≠", "approx": "≈", "equiv": "≡", "sim": "∼", "propto": "∝", "infty": "∞",
        "sum": "∑", "prod": "∏", "int": "∫", "oint": "∮", "partial": "∂", "nabla": "∇", "to": "→",
        "rightarrow": "→", "leftarrow": "←", "Rightarrow": "⇒", "Leftarrow": "⇐", "implies": "⇒",
        "iff": "⇔", "leftrightarrow": "↔", "mapsto": "↦", "in": "∈", "notin": "∉", "subset": "⊂",
        "subseteq": "⊆", "supset": "⊃", "cup": "∪", "cap": "∩", "emptyset": "∅", "forall": "∀",
        "exists": "∃", "neg": "¬", "land": "∧", "lor": "∨", "wedge": "∧", "vee": "∨", "circ": "∘",
        "degree": "°", "ldots": "…", "cdots": "⋯", "dots": "…", "prime": "′", "angle": "∠", "perp": "⊥",
        "parallel": "∥", "langle": "⟨", "rangle": "⟩", "lfloor": "⌊", "rfloor": "⌋", "lceil": "⌈",
        "rceil": "⌉", "hbar": "ħ", "ell": "ℓ", "star": "⋆", "ast": "∗", "mid": "∣", "lvert": "|", "rvert": "|",
        "vert": "|", "sin": "sin", "cos": "cos", "tan": "tan", "log": "log", "ln": "ln", "exp": "exp",
        "lim": "lim", "max": "max", "min": "min",
        "alpha": "α", "beta": "β", "gamma": "γ", "delta": "δ", "epsilon": "ε", "varepsilon": "ε", "zeta": "ζ",
        "eta": "η", "theta": "θ", "vartheta": "ϑ", "iota": "ι", "kappa": "κ", "lambda": "λ", "mu": "μ",
        "nu": "ν", "xi": "ξ", "pi": "π", "rho": "ρ", "sigma": "σ", "tau": "τ", "upsilon": "υ", "phi": "φ",
        "varphi": "φ", "chi": "χ", "psi": "ψ", "omega": "ω", "Gamma": "Γ", "Delta": "Δ", "Theta": "Θ",
        "Lambda": "Λ", "Xi": "Ξ", "Pi": "Π", "Sigma": "Σ", "Phi": "Φ", "Psi": "Ψ", "Omega": "Ω",
    ]

    static let superscripts: [Character: Character] = [
        "0": "⁰", "1": "¹", "2": "²", "3": "³", "4": "⁴", "5": "⁵", "6": "⁶", "7": "⁷", "8": "⁸", "9": "⁹",
        "+": "⁺", "-": "⁻", "=": "⁼", "(": "⁽", ")": "⁾", "n": "ⁿ", "i": "ⁱ", "x": "ˣ", "y": "ʸ", "k": "ᵏ",
        "T": "ᵀ", "′": "′",
    ]

    static let subscripts: [Character: Character] = [
        "0": "₀", "1": "₁", "2": "₂", "3": "₃", "4": "₄", "5": "₅", "6": "₆", "7": "₇", "8": "₈", "9": "₉",
        "+": "₊", "-": "₋", "=": "₌", "(": "₍", ")": "₎", "a": "ₐ", "e": "ₑ", "i": "ᵢ", "j": "ⱼ", "k": "ₖ",
        "n": "ₙ", "o": "ₒ", "x": "ₓ", "m": "ₘ", "t": "ₜ",
    ]

    static let doubleStruck: [Character: Character] = [
        "R": "ℝ", "N": "ℕ", "Z": "ℤ", "Q": "ℚ", "C": "ℂ",
    ]
}
