use crate::banking::{circle, mercury};
use crate::chains::{aptos, evm, near, solana, starknet, sui};
use crate::storage::{BankingService, Chain};
use crate::types::FetchFailure;

fn format_usd(value: f64) -> String {
    let formatted = format!("{:.2}", value);
    let parts: Vec<&str> = formatted.split('.').collect();
    let integer_part = parts[0];
    let decimal_part = parts.get(1).unwrap_or(&"00");

    // Handle negative sign separately
    let (sign, digits) = if let Some(d) = integer_part.strip_prefix('-') {
        ("-", d)
    } else {
        ("", integer_part)
    };

    // Add commas to digit part
    let mut result = String::new();
    for (i, ch) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }

    format!(
        "{}{}.{}",
        sign,
        result.chars().rev().collect::<String>(),
        decimal_part
    )
}

pub fn render_solana_balances(
    company: &str,
    name: &str,
    address: &str,
    balances: &solana::AccountBalances,
    chain: &Chain,
) {
    const MIN_WIDTH: usize = 79;

    // Collect all content lines to calculate max width
    let mut lines = Vec::new();

    // Header lines
    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", &address[..42.min(address.len())]));
    lines.push(format!("Chain: {}", chain.display_name()));

    // SOL Balance line
    let sol_line = if let Some(usd_value) = balances.sol_usd_value {
        if let Some(price) = balances.sol_usd_price {
            format!(
                "SOL Balance: {:.9} SOL (${} @ ${})",
                balances.sol_balance,
                format_usd(usd_value),
                format_usd(price)
            )
        } else {
            format!(
                "SOL Balance: {:.9} SOL (${})",
                balances.sol_balance,
                format_usd(usd_value)
            )
        }
    } else {
        format!("SOL Balance: {:.9} SOL", balances.sol_balance)
    };
    lines.push(sol_line);

    // Token balance lines
    if !balances.token_balances.is_empty() {
        lines.push("TOKEN BALANCES".to_string());
        for token in &balances.token_balances {
            let token_display = match (&token.name, &token.symbol) {
                (Some(name), Some(symbol)) => format!("{} ({})", name, symbol),
                (Some(name), None) => name.clone(),
                (None, Some(symbol)) => symbol.clone(),
                (None, None) => "Unknown Token".to_string(),
            };
            lines.push(token_display);

            let mint_display = if token.mint.len() > 44 {
                format!(
                    "    Mint: {}...{}",
                    &token.mint[..20],
                    &token.mint[token.mint.len() - 20..]
                )
            } else {
                format!("    Mint: {}", token.mint)
            };
            lines.push(mint_display);

            let mut balance_str = if let Some(usd_value) = token.usd_value {
                if let Some(price) = token.usd_price {
                    format!(
                        "    Balance: {:.6} (${} @ ${:.6})",
                        token.ui_amount,
                        format_usd(usd_value),
                        price
                    )
                } else {
                    format!(
                        "    Balance: {:.6} (${})",
                        token.ui_amount,
                        format_usd(usd_value)
                    )
                }
            } else {
                format!("    Balance: {:.6}", token.ui_amount)
            };
            balance_str.push_str(&solana::supply_suffix(token.supply_percent));
            lines.push(balance_str);
            lines.push(format!("    Decimals: {}", token.decimals));
        }
    }

    // Total USD Value line
    if let Some(total) = balances.total_usd_value {
        lines.push(format!("TOTAL USD VALUE: ${}", format_usd(total)));
    }

    // Calculate max width needed
    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    // Top border
    println!("\n╔{}╗", "═".repeat(box_width + 2));

    // Header section
    println!("║  {:<width$} ║", lines[0], width = box_width);
    println!("║  {:<width$} ║", lines[1], width = box_width);
    println!("║  {:<width$} ║", lines[2], width = box_width);
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // SOL Balance
    println!("║  {:<width$} ║", lines[4], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // Token Balances
    if balances.token_balances.is_empty() {
        println!("║  {:<width$} ║", "Token Balances: None", width = box_width);
    } else {
        println!("║  {:<width$} ║", lines[5], width = box_width);
        println!("╟{}╢", "─".repeat(box_width + 2));

        let mut line_idx = 6;
        for _ in &balances.token_balances {
            println!("║  {:<width$} ║", lines[line_idx], width = box_width); // Token name
            println!("║  {:<width$} ║", lines[line_idx + 1], width = box_width); // Mint
            println!("║  {:<width$} ║", lines[line_idx + 2], width = box_width); // Balance
            println!("║  {:<width$} ║", lines[line_idx + 3], width = box_width); // Decimals
            println!("╟{}╢", "─".repeat(box_width + 2));
            line_idx += 4;
        }
    }

    // Total USD Value
    if balances.total_usd_value.is_some() {
        let total_line_idx = lines.len() - 1;
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[total_line_idx], width = box_width);
    }

    // Bottom border
    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_error(error: &str) {
    println!(
        "\n╭─────────────────────────────────────────────────────────────────────────────────╮"
    );
    println!(
        "│ ERROR                                                                            │"
    );
    println!("├─────────────────────────────────────────────────────────────────────────────────┤");
    println!(
        "│ {}                                                                              │",
        error
    );
    println!(
        "╰─────────────────────────────────────────────────────────────────────────────────╯\n"
    );
}

pub fn render_success(message: &str) {
    println!("\n{}\n", message);
}

pub fn render_evm_balances(
    company: &str,
    name: &str,
    address: &str,
    balances: &evm::AccountBalances,
    chain: &Chain,
) {
    const MIN_WIDTH: usize = 79;

    // Collect all content lines to calculate max width
    let mut lines = Vec::new();

    // Header lines
    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    // Native token balance line (ETH, CORE, MATIC, BNB, AVAX, etc.)
    let native_symbol = chain.native_token_symbol();
    let native_line = if let Some(usd_value) = balances.eth_usd_value {
        if let Some(price) = balances.eth_usd_price {
            format!(
                "{} Balance: {:.9} {} (${} @ ${})",
                native_symbol,
                balances.eth_balance,
                native_symbol,
                format_usd(usd_value),
                format_usd(price)
            )
        } else {
            format!(
                "{} Balance: {:.9} {} (${})",
                native_symbol,
                balances.eth_balance,
                native_symbol,
                format_usd(usd_value)
            )
        }
    } else {
        format!(
            "{} Balance: {:.9} {}",
            native_symbol, balances.eth_balance, native_symbol
        )
    };
    lines.push(native_line);

    // Token balance lines
    if !balances.token_balances.is_empty() {
        lines.push("ERC20 TOKEN BALANCES".to_string());
        for token in &balances.token_balances {
            let token_display = match (&token.name, &token.symbol) {
                (Some(name), Some(symbol)) => format!("{} ({})", name, symbol),
                (Some(name), None) => name.clone(),
                (None, Some(symbol)) => symbol.clone(),
                (None, None) => "Unknown Token".to_string(),
            };
            lines.push(token_display);

            lines.push(format!("    Contract: {}", token.contract_address));

            let balance_str = if let Some(usd_value) = token.usd_value {
                if let Some(price) = token.usd_price {
                    format!(
                        "    Balance: {:.6} (${} @ ${:.6})",
                        token.ui_amount,
                        format_usd(usd_value),
                        price
                    )
                } else {
                    format!(
                        "    Balance: {:.6} (${})",
                        token.ui_amount,
                        format_usd(usd_value)
                    )
                }
            } else {
                format!("    Balance: {:.6}", token.ui_amount)
            };
            lines.push(balance_str);
            lines.push(format!("    Decimals: {}", token.decimals));
        }
    }

    // Total USD Value line
    if let Some(total) = balances.total_usd_value {
        lines.push(format!("TOTAL USD VALUE: ${}", format_usd(total)));
    }

    // Calculate max width needed
    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    // Top border
    println!("\n╔{}╗", "═".repeat(box_width + 2));

    // Header section
    println!("║  {:<width$} ║", lines[0], width = box_width);
    println!("║  {:<width$} ║", lines[1], width = box_width);
    println!("║  {:<width$} ║", lines[2], width = box_width);
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // ETH Balance
    println!("║  {:<width$} ║", lines[4], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // Token Balances
    if balances.token_balances.is_empty() {
        println!("║  {:<width$} ║", "Token Balances: None", width = box_width);
    } else {
        println!("║  {:<width$} ║", lines[5], width = box_width);
        println!("╟{}╢", "─".repeat(box_width + 2));

        let mut line_idx = 6;
        for _ in &balances.token_balances {
            println!("║  {:<width$} ║", lines[line_idx], width = box_width); // Token name
            println!("║  {:<width$} ║", lines[line_idx + 1], width = box_width); // Contract
            println!("║  {:<width$} ║", lines[line_idx + 2], width = box_width); // Balance
            println!("║  {:<width$} ║", lines[line_idx + 3], width = box_width); // Decimals
            println!("╟{}╢", "─".repeat(box_width + 2));
            line_idx += 4;
        }
    }

    // Total USD Value
    if balances.total_usd_value.is_some() {
        let total_line_idx = lines.len() - 1;
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[total_line_idx], width = box_width);
    }

    // Bottom border
    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_portfolio_summary(
    portfolio: &crate::types::PortfolioSummary,
    price_info: Option<&str>,
) {
    const BOX_WIDTH: usize = 81;

    println!(
        "\n╔═════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!("║                               PORTFOLIO SUMMARY                                 ║");
    println!("╠═════════════════════════════════════════════════════════════════════════════════╣");

    // Total Portfolio Value with proper padding
    let total_value_str = format!(
        "Total Portfolio Value: ${}",
        format_usd(portfolio.total_usd_value)
    );
    let total_value_len = total_value_str.len();
    let total_padding = (BOX_WIDTH - 2).saturating_sub(total_value_len);
    println!(
        "║  {}{:width$} ║",
        total_value_str,
        "",
        width = total_padding
    );

    // Show price cache info if present
    if let Some(info) = price_info {
        let info_len = info.len();
        let info_padding = (BOX_WIDTH - 2).saturating_sub(info_len);
        println!("║  {}{:width$} ║", info, "", width = info_padding);
    }

    if portfolio.companies.is_empty() {
        println!(
            "╠═════════════════════════════════════════════════════════════════════════════════╣"
        );
        println!(
            "║  No companies found                                                             ║"
        );
        println!(
            "╚═════════════════════════════════════════════════════════════════════════════════╝\n"
        );
        return;
    }

    // Sort companies by USD value (descending)
    let mut sorted_companies: Vec<_> = portfolio.companies.iter().collect();
    sorted_companies.sort_by(|a, b| b.1.total_usd_value.total_cmp(&a.1.total_usd_value));

    for (company_name, company) in sorted_companies {
        println!(
            "╠═════════════════════════════════════════════════════════════════════════════════╣"
        );

        // Company header
        let company_header = format!("COMPANY: {}", company_name);
        let company_header_len = company_header.len();
        let company_padding = (BOX_WIDTH - 2).saturating_sub(company_header_len);
        println!(
            "║  {}{:width$} ║",
            company_header,
            "",
            width = company_padding
        );

        // Company total value
        let company_value_str = format!("Total Value: ${}", format_usd(company.total_usd_value));
        let company_value_len = company_value_str.len();
        let company_value_padding = if company_value_len + 2 < BOX_WIDTH - 2 {
            BOX_WIDTH - 2 - company_value_len - 2
        } else {
            0
        };
        println!(
            "║    {}{:width$} ║",
            company_value_str,
            "",
            width = company_value_padding
        );

        println!(
            "╟─────────────────────────────────────────────────────────────────────────────────╢"
        );

        if company.assets.is_empty() {
            println!("║      No assets found                                                            ║");
        } else {
            // Sort assets by USD value (descending)
            let mut sorted_assets: Vec<_> = company.assets.iter().collect();
            sorted_assets.sort_by(|a, b| {
                let a_val = a.1.usd_value.unwrap_or(0.0);
                let b_val = b.1.usd_value.unwrap_or(0.0);
                b_val.total_cmp(&a_val)
            });

            for (_, asset) in sorted_assets {
                // Symbol line
                let symbol_str = format!("{}:", asset.symbol);
                let symbol_len = symbol_str.len();
                let symbol_padding = if symbol_len + 4 < BOX_WIDTH - 2 {
                    BOX_WIDTH - 2 - symbol_len - 4
                } else {
                    0
                };
                println!(
                    "║      {}{:width$} ║",
                    symbol_str,
                    "",
                    width = symbol_padding
                );

                // Amount and USD Value on same line if USD value exists
                let usd_value = asset.usd_value.unwrap_or(0.0);
                if usd_value > 0.0 {
                    let detail_str = format!("{:.6} (${:})", asset.amount, format_usd(usd_value));
                    let detail_len = detail_str.len();
                    let detail_padding = if detail_len + 8 < BOX_WIDTH - 2 {
                        BOX_WIDTH - 2 - detail_len - 8
                    } else {
                        0
                    };
                    println!(
                        "║          {}{:width$} ║",
                        detail_str,
                        "",
                        width = detail_padding
                    );
                } else {
                    let amount_str = format!("{:.6}", asset.amount);
                    let amount_len = amount_str.len();
                    let amount_padding = if amount_len + 8 < BOX_WIDTH - 2 {
                        BOX_WIDTH - 2 - amount_len - 8
                    } else {
                        0
                    };
                    println!(
                        "║          {}{:width$} ║",
                        amount_str,
                        "",
                        width = amount_padding
                    );
                }
            }
        }
    }

    println!(
        "╚═════════════════════════════════════════════════════════════════════════════════╝\n"
    );
}

#[allow(clippy::too_many_arguments)]
fn render_simple_balance(
    company: &str,
    name: &str,
    address: &str,
    chain: &Chain,
    symbol: &str,
    balance: f64,
    usd_price: Option<f64>,
    usd_value: Option<f64>,
    total_usd_value: Option<f64>,
) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    let balance_line = if let Some(usd_val) = usd_value {
        if let Some(price) = usd_price {
            format!(
                "{} Balance: {:.9} {} (${} @ ${})",
                symbol,
                balance,
                symbol,
                format_usd(usd_val),
                format_usd(price)
            )
        } else {
            format!(
                "{} Balance: {:.9} {} (${})",
                symbol,
                balance,
                symbol,
                format_usd(usd_val)
            )
        }
    } else {
        format!("{} Balance: {:.9} {}", symbol, balance, symbol)
    };
    lines.push(balance_line);

    if let Some(total) = total_usd_value {
        lines.push(format!("TOTAL USD VALUE: ${}", format_usd(total)));
    }

    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    println!("\n╔{}╗", "═".repeat(box_width + 2));
    println!("║  {:<width$} ║", lines[0], width = box_width);
    println!("║  {:<width$} ║", lines[1], width = box_width);
    println!("║  {:<width$} ║", lines[2], width = box_width);
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));
    println!("║  {:<width$} ║", lines[4], width = box_width);

    if total_usd_value.is_some() {
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[5], width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_near_balances(
    company: &str,
    name: &str,
    address: &str,
    balances: &near::AccountBalances,
    chain: &Chain,
) {
    render_simple_balance(
        company,
        name,
        address,
        chain,
        "NEAR",
        balances.near_balance,
        balances.near_usd_price,
        balances.near_usd_value,
        balances.total_usd_value,
    );
}

pub fn render_aptos_balances(
    company: &str,
    name: &str,
    address: &str,
    balances: &aptos::AccountBalances,
    chain: &Chain,
) {
    render_simple_balance(
        company,
        name,
        address,
        chain,
        "APT",
        balances.apt_balance,
        balances.apt_usd_price,
        balances.apt_usd_value,
        balances.total_usd_value,
    );
}

pub fn render_sui_balances(
    company: &str,
    name: &str,
    address: &str,
    balances: &sui::AccountBalances,
    chain: &Chain,
) {
    render_simple_balance(
        company,
        name,
        address,
        chain,
        "SUI",
        balances.sui_balance,
        balances.sui_usd_price,
        balances.sui_usd_value,
        balances.total_usd_value,
    );
}

pub fn render_starknet_balances(
    company: &str,
    name: &str,
    address: &str,
    balances: &starknet::AccountBalances,
    chain: &Chain,
) {
    render_simple_balance(
        company,
        name,
        address,
        chain,
        "ETH",
        balances.eth_balance,
        balances.eth_usd_price,
        balances.eth_usd_value,
        balances.total_usd_value,
    );
}

pub fn render_mercury_balances(
    company: &str,
    name: &str,
    account_id: &str,
    balances: &mercury::AccountBalances,
    service: &BankingService,
) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Account: {}", name));
    lines.push(format!("Account ID: {}", account_id));
    lines.push(format!("Service: {}", service.display_name()));
    lines.push(format!("Status: {}", balances.status));

    lines.push(format!(
        "Available Balance: ${}",
        format_usd(balances.available_balance)
    ));
    lines.push(format!(
        "Current Balance: ${}",
        format_usd(balances.current_balance)
    ));

    if let Some(created_at) = &balances.created_at {
        lines.push(format!("Created: {}", created_at));
    }

    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    println!("\n╔{}╗", "═".repeat(box_width + 2));
    println!("║  {:<width$} ║", lines[0], width = box_width);
    println!("║  {:<width$} ║", lines[1], width = box_width);
    println!("║  {:<width$} ║", lines[2], width = box_width);
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("║  {:<width$} ║", lines[4], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));
    println!("║  {:<width$} ║", lines[5], width = box_width);
    println!("║  {:<width$} ║", lines[6], width = box_width);

    if balances.created_at.is_some() {
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[7], width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_circle_balances(
    company: &str,
    name: &str,
    balances: &circle::AccountBalances,
    service: &BankingService,
) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Account: {}", name));
    lines.push(format!("Service: {}", service.display_name()));

    // Available balances section
    if !balances.available_balances.is_empty() {
        lines.push("AVAILABLE BALANCES".to_string());
        for balance in &balances.available_balances {
            let currency_display = if balance.currency == "USD" {
                "USDC"
            } else if balance.currency == "EUR" {
                "EURC"
            } else {
                &balance.currency
            };
            lines.push(format!(
                "  {}: ${}",
                currency_display,
                format_usd(balance.amount)
            ));
        }
    }

    // Unsettled balances section
    if !balances.unsettled_balances.is_empty() {
        let has_nonzero_unsettled = balances.unsettled_balances.iter().any(|b| b.amount > 0.0);
        if has_nonzero_unsettled {
            lines.push("UNSETTLED BALANCES (pending)".to_string());
            for balance in &balances.unsettled_balances {
                if balance.amount > 0.0 {
                    let currency_display = if balance.currency == "USD" {
                        "USDC"
                    } else if balance.currency == "EUR" {
                        "EURC"
                    } else {
                        &balance.currency
                    };
                    lines.push(format!(
                        "  {}: ${}",
                        currency_display,
                        format_usd(balance.amount)
                    ));
                }
            }
        }
    }

    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    println!("\n╔{}╗", "═".repeat(box_width + 2));

    // Print header lines
    for line in lines.iter().take(3) {
        println!("║  {:<width$} ║", line, width = box_width);
    }

    println!("╠{}╣", "═".repeat(box_width + 2));

    // Print remaining lines
    for line in lines.iter().skip(3) {
        println!("║  {:<width$} ║", line, width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

/// Renders a manual account: the hand-entered balance plus the date it was
/// entered, so staleness is visible at a glance.
pub fn render_manual_balance(account: &crate::storage::BankingAccount) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if account.company.is_empty() {
        "-"
    } else {
        &account.company
    };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Account: {}", account.name));
    lines.push(format!("Service: {}", account.service.display_name()));

    match (&account.manual_balance, account.manual_as_of()) {
        (Some(manual), Some(as_of)) => lines.push(format!(
            "Balance: ${} (as of {})",
            format_usd(manual.amount),
            as_of
        )),
        _ => lines.push(format!(
            "Balance: not set - use 'gringotts set-balance \"{}\" <amount>'",
            account.name
        )),
    }

    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    println!("\n╔{}╗", "═".repeat(box_width + 2));
    for line in lines.iter().take(3) {
        println!("║  {:<width$} ║", line, width = box_width);
    }
    println!("╠{}╣", "═".repeat(box_width + 2));
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

/// Renders a summary of failed wallet/account queries
pub fn render_fetch_failures(failures: &[FetchFailure]) {
    if failures.is_empty() {
        return;
    }

    const MIN_WIDTH: usize = 60;
    let mut lines = Vec::new();

    lines.push(format!("FAILED QUERIES ({} total)", failures.len()));
    lines.push(String::new());

    for failure in failures {
        lines.push(format!("  {} ({})", failure.name, failure.chain_or_service));
        // Truncate error message if too long
        let error_display = if failure.error.len() > 50 {
            format!("{}...", &failure.error[..47])
        } else {
            failure.error.clone()
        };
        lines.push(format!("    Error: {}", error_display));
    }

    lines.push(String::new());
    lines.push("Note: Cached data preserved for failed queries".to_string());

    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    println!("\n╔{}╗", "═".repeat(box_width + 2));

    for line in &lines {
        if line.is_empty() {
            println!("║  {:width$} ║", "", width = box_width);
        } else {
            println!("║  {:<width$} ║", line, width = box_width);
        }
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}
