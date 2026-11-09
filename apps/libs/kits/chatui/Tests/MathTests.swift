import TestingKit

@testable import ChatUI

func mathTests() -> [TestCase] {
    [
        TestCase("given LaTeX, then its symbols") {
            try expectEqual(Math.text(#"\frac{a}{b} \times \pi r^2"#), "a/b × π r²")
            try expectEqual(Math.text(#"\sqrt{x+1} \leq \alpha_1"#), "√(x+1) ≤ α₁")
            try expectEqual(Math.text(#"e^{-x}"#), "e⁻ˣ")
        },
        TestCase("given a script with no small letters, then it's written after its mark") {
            try expectEqual(Math.text(#"x^{ab}"#), "x^(ab)")
            try expectEqual(Math.text(#"x_q"#), "x_q")
        },
        TestCase("given words in maths, then the words") {
            try expectEqual(Math.text(#"\text{area} = \left( w \cdot h \right)"#), "area = ( w · h )")
        },
        TestCase("given inline maths, then its symbols, which Markdown doesn't read") {
            try expectEqual(Math.inline(#"So $x^2 + y_1$ and \(\beta\)."#), #"So x² + y₁ and β."#)
            try expectEqual(Math.inline(#"$a_b$"#), #"a\_b"#)
        },
        TestCase("given prices or code, then no maths") {
            try expectEqual(Math.inline("It's $5 and $10."), "It's $5 and $10.")
            try expectEqual(Math.inline("Costs $5, or $ 6$."), "Costs $5, or $ 6$.")
            try expectEqual(Math.inline("Run `echo $HOME$` now"), "Run `echo $HOME$` now")
        },
        TestCase("given $$ on lines of their own, then a block of maths between paragraphs") {
            try expectEqual(
                Markdown.blocks(in: "Area:\n$$\nA = \\pi r^2\n$$\nDone."),
                [.paragraph("Area:"), .math("A = \\pi r^2"), .paragraph("Done.")])
        },
        TestCase("given \\[ and its closing on one line, then a block of maths") {
            try expectEqual(Markdown.blocks(in: "\\[ E = mc^2 \\]"), [.math("E = mc^2")])
        },
        TestCase("given maths still being written, then what's come so far") {
            try expectEqual(Markdown.blocks(in: "$$\nx = \\frac{1}"), [.math("x = \\frac{1}")])
        },
    ]
}
