import TestingKit

@testable import ChatUI

func textScaleTests() -> [TestCase] {
    [
        TestCase("given the usual size, then a step either way is a tenth of it") {
            try expectEqual(TextScale.actual.bigger.percent, 110)
            try expectEqual(TextScale.actual.smaller.percent, 90)
            try expectEqual(TextScale.actual.bigger.factor, 1.1)
        },
        TestCase("given steps up and as many back down, then the usual size again") {
            var scale = TextScale.actual
            for _ in 0..<4 { scale = scale.bigger }
            for _ in 0..<4 { scale = scale.smaller }
            try expectEqual(scale, .actual)
        },
        TestCase("given the biggest, then it grows no more and still shrinks") {
            var scale = TextScale.actual
            for _ in 0..<40 { scale = scale.bigger }
            try expectEqual(scale.percent, TextScale.percents.upperBound)
            try expectEqual(scale.canGrow, false)
            try expectEqual(scale.bigger, scale)
            try expectEqual(scale.canShrink, true)
        },
        TestCase("given the smallest, then it shrinks no more and still grows") {
            var scale = TextScale.actual
            for _ in 0..<40 { scale = scale.smaller }
            try expectEqual(scale.percent, TextScale.percents.lowerBound)
            try expectEqual(scale.canShrink, false)
            try expectEqual(scale.smaller, scale)
            try expectEqual(scale.canGrow, true)
        },
        TestCase("given a percentage between steps or past the limits, then the nearest scale there is") {
            try expectEqual(TextScale(percent: 133).percent, 130)
            try expectEqual(TextScale(percent: 137).percent, 140)
            try expectEqual(TextScale(percent: 1_000).percent, TextScale.percents.upperBound)
            try expectEqual(TextScale(percent: -20).percent, TextScale.percents.lowerBound)
        },
    ]
}
