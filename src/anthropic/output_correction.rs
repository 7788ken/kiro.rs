//! 缓存差价补正：把扣费与刊例价的差额折算成输出 token
//!
//! 开启 prompt cache 后，缓存读取按官方刊例只算普通输入的约 1/10，
//! 而我们的扣费锚定凭证真实消耗，上游并不因为"这些 token 是缓存命中"而少扣我们，
//! 于是客户端按刊例算出的金额远低于实际扣费，形成成本倒挂。
//!
//! 这里把差额折算成输出 token 补上：输入与缓存三项保持真实，只抬高上报的输出。
//! 补正只改上报给客户端的 usage，不影响扣费。

use crate::accounts::models::BillingConfig;

use super::prompt_cache::UsageTokens;

/// 官方刊例的相对价（以普通输入为 1）
///
/// Anthropic 的 prompt cache 定价：写入 1.25 倍，读取 0.1 倍；
/// 输出相对输入的倍数按 `BillingConfig` 的实际单价计算，不写死。
const CACHE_WRITE_RELATIVE: f64 = 1.25;
const CACHE_READ_RELATIVE: f64 = 0.1;

/// 补正所需的本次请求成本信息
#[derive(Debug, Clone, Copy)]
pub struct CorrectionInput {
    /// 实际向用户收取的积分（凭证消耗 × 消费倍率），单位：积分
    pub charged_credits: f64,
    /// 上报的真实输出 token
    pub output_tokens: i32,
}

/// 把扣费与刊例价的差额折算成输出 token，返回补正后的输出 token。
///
/// 按 `billing` 的单价算出客户端照 usage 自行估算的金额，
/// 若实际扣费更高，就把差额按输出单价折成 token 补进输出。
/// 扣费不高于刊例估算时不补，返回真实输出。
///
/// `target_ratio` 是补正目标倍数：1 表示补到与扣费持平，
/// 1.2 表示补到扣费的 1.2 倍（留出毛利展示空间）。
pub fn corrected_output_tokens(
    usage: &UsageTokens,
    input: CorrectionInput,
    billing: &BillingConfig,
    target_ratio: f64,
) -> i32 {
    let output_price = billing.output_per_million;
    // 输出单价为 0 时无法折算，直接返回真实输出
    if !output_price.is_finite() || output_price <= 0.0 {
        return input.output_tokens;
    }
    if !input.charged_credits.is_finite() || input.charged_credits <= 0.0 {
        return input.output_tokens;
    }
    let ratio = if target_ratio.is_finite() && target_ratio > 0.0 {
        target_ratio
    } else {
        1.0
    };

    // 客户端按 usage 和刊例单价能算出的金额（与扣费同一量纲：积分）
    let input_price = billing.input_per_million;
    let listed = f64::from(usage.input_tokens) * input_price
        + f64::from(usage.cache_creation_input_tokens) * input_price * CACHE_WRITE_RELATIVE
        + f64::from(usage.cache_read_input_tokens) * input_price * CACHE_READ_RELATIVE
        + f64::from(input.output_tokens) * output_price;

    // 单价是"积分/百万 token"，所以金额要除以百万
    let listed_credits = listed / 1_000_000.0;
    let target_credits = input.charged_credits * ratio;
    if !listed_credits.is_finite() || listed_credits >= target_credits {
        return input.output_tokens;
    }

    // 差额折成输出 token
    let gap_credits = target_credits - listed_credits;
    let extra_tokens = gap_credits * 1_000_000.0 / output_price;
    if !extra_tokens.is_finite() || extra_tokens <= 0.0 {
        return input.output_tokens;
    }
    let extra = extra_tokens.ceil().min(f64::from(i32::MAX)) as i64;
    (i64::from(input.output_tokens).saturating_add(extra)).min(i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn billing() -> BillingConfig {
        BillingConfig {
            input_per_million: 10.0,
            output_per_million: 50.0,
            consumption_ratio: 1.0,
        }
    }

    fn usage(input: i32, creation: i32, read: i32) -> UsageTokens {
        UsageTokens {
            input_tokens: input,
            cache_creation_input_tokens: creation,
            cache_read_input_tokens: read,
        }
    }

    #[test]
    fn correction_folds_gap_into_output() {
        // 线上典型值：普通输入 26737，缓存写 1656，缓存读 11758，输出 161
        let u = usage(26737, 1656, 11758);
        let billing = billing();
        // 刊例估算：按上面的相对价算出的金额
        let listed = (26737.0 * 10.0 + 1656.0 * 12.5 + 11758.0 * 1.0 + 161.0 * 50.0) / 1e6;
        // 扣费设为刊例的 1.5 倍（线上实测整体缺口）
        let charged = listed * 1.5;
        let corrected = corrected_output_tokens(
            &u,
            CorrectionInput {
                charged_credits: charged,
                output_tokens: 161,
            },
            &billing,
            1.0,
        );
        assert!(corrected > 161, "应补高输出，实际 {corrected}");
        // 补正后按刊例重算，应约等于扣费
        let after =
            (26737.0 * 10.0 + 1656.0 * 12.5 + 11758.0 * 1.0 + f64::from(corrected) * 50.0) / 1e6;
        assert!(
            (after - charged).abs() / charged < 0.01,
            "补正后刊例 {after} 与扣费 {charged} 偏差过大"
        );
    }

    #[test]
    fn no_correction_when_charge_below_listed() {
        let u = usage(1000, 0, 0);
        let listed = 1000.0 * 10.0 / 1e6;
        let corrected = corrected_output_tokens(
            &u,
            CorrectionInput {
                charged_credits: listed * 0.5,
                output_tokens: 100,
            },
            &billing(),
            1.0,
        );
        assert_eq!(corrected, 100, "扣费低于刊例时不应补");
    }

    #[test]
    fn target_ratio_scales_correction() {
        let u = usage(10_000, 0, 50_000);
        let billing = billing();
        let base = corrected_output_tokens(
            &u,
            CorrectionInput {
                charged_credits: 10.0,
                output_tokens: 100,
            },
            &billing,
            1.0,
        );
        let higher = corrected_output_tokens(
            &u,
            CorrectionInput {
                charged_credits: 10.0,
                output_tokens: 100,
            },
            &billing,
            1.5,
        );
        assert!(higher > base, "目标倍数越高补得越多：{base} vs {higher}");
    }

    #[test]
    fn invalid_inputs_return_real_output() {
        let u = usage(1000, 0, 5000);
        let b = billing();
        let cases = [f64::NAN, f64::INFINITY, -1.0, 0.0];
        for charged in cases {
            let corrected = corrected_output_tokens(
                &u,
                CorrectionInput {
                    charged_credits: charged,
                    output_tokens: 42,
                },
                &b,
                1.0,
            );
            assert_eq!(corrected, 42, "charged={charged} 应原样返回");
        }
        // 输出单价为 0 时无法折算
        let zero_price = BillingConfig {
            input_per_million: 10.0,
            output_per_million: 0.0,
            consumption_ratio: 1.0,
        };
        assert_eq!(
            corrected_output_tokens(
                &u,
                CorrectionInput {
                    charged_credits: 99.0,
                    output_tokens: 42
                },
                &zero_price,
                1.0
            ),
            42
        );
        // 目标倍数非法时按 1 处理，不 panic
        for ratio in [f64::NAN, 0.0, -2.0] {
            let corrected = corrected_output_tokens(
                &u,
                CorrectionInput {
                    charged_credits: 99.0,
                    output_tokens: 42,
                },
                &b,
                ratio,
            );
            assert!(corrected >= 42, "ratio={ratio} 不应低于真实输出");
        }
    }

    #[test]
    fn tiny_output_can_still_be_corrected() {
        // 纯缓存命中、模型立即返回的场景：真实输出极小，仍应能补
        let u = usage(100, 0, 200_000);
        let corrected = corrected_output_tokens(
            &u,
            CorrectionInput {
                charged_credits: 5.0,
                output_tokens: 1,
            },
            &billing(),
            1.0,
        );
        assert!(corrected > 1, "输出为 1 时也应补，实际 {corrected}");
    }
}
