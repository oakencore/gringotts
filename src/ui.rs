use crate::aptos;
use crate::evm;
use crate::near;
use crate::solana;
use crate::starknet;
use crate::storage::{Chain, WalletAddress};
use crate::sui;

fn format_usd(value: f64) -> String {
    let formatted = format!("{:.2}", value);
    let parts: Vec<&str> = formatted.split('.').collect();
    let integer_part = parts[0];
    let decimal_part = parts.get(1).unwrap_or(&"00");

    // Add commas to integer part
    let mut result = String::new();
    for (i, ch) in integer_part.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }

    format!("{}.{}", result.chars().rev().collect::<String>(), decimal_part)
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        s.chars().take(max_len).collect()
    } else {
        let prefix_len = (max_len - 3) / 2;
        let suffix_len = max_len - 3 - prefix_len;
        format!("{}...{}",
            s.chars().take(prefix_len).collect::<String>(),
            s.chars().skip(s.chars().count() - suffix_len).collect::<String>()
        )
    }
}

pub fn render_addresses(addresses: &[WalletAddress]) {
    if addresses.is_empty() {
        println!("\nNo addresses tracked yet. Use 'gringotts add' to add addresses.\n");
        return;
    }

    // Get terminal width, default to 120 if detection fails
    let term_width = if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        w as usize
    } else {
        120
    };

    // Calculate column widths based on terminal size
    // Minimum: 8 chars for borders and separators (│ X │ X │ X │ X │)
    let available_width = term_width.saturating_sub(8);

    // Set minimum widths for each column
    let min_company = 8;
    let min_name = 15;
    let min_address = 20;
    let min_chain = 10;
    let min_total = min_company + min_name + min_address + min_chain;

    let (company_width, name_width, address_width, chain_width) = if available_width < min_total {
        // If terminal is too small, use minimum widths
        (min_company, min_name, min_address, min_chain)
    } else {
        // Distribute extra space proportionally
        let extra = available_width - min_total;
        // Give more space to Name and Address columns
        let company_w = min_company + (extra * 1) / 10;
        let name_w = min_name + (extra * 3) / 10;
        let address_w = min_address + (extra * 5) / 10;
        let chain_w = min_chain + (extra * 1) / 10;
        (company_w, name_w, address_w, chain_w)
    };

    let table_width = company_width + name_width + address_width + chain_width + 8;

    // Print header
    println!("\n╭{}╗", "─".repeat(table_width - 2));
    let title = "TRACKED ADDRESSES";
    let title_padding = (table_width - 2 - title.len()) / 2;
    println!("│{}{:^width$}{}│",
        " ".repeat(title_padding),
        title,
        " ".repeat(table_width - 2 - title_padding - title.len()),
        width = title.len()
    );
    println!("├{}┬{}┬{}┬{}┤",
        "─".repeat(company_width),
        "─".repeat(name_width),
        "─".repeat(address_width),
        "─".repeat(chain_width)
    );

    // Print column headers
    println!("│{:^cw$}│{:^nw$}│{:^aw$}│{:^chw$}│",
        "Company", "Name", "Address", "Chain",
        cw = company_width, nw = name_width, aw = address_width, chw = chain_width
    );
    println!("├{}┼{}┼{}┼{}┤",
        "─".repeat(company_width),
        "─".repeat(name_width),
        "─".repeat(address_width),
        "─".repeat(chain_width)
    );

    // Print addresses
    for addr in addresses {
        let display_company = if addr.company.is_empty() {
            "-".to_string()
        } else {
            truncate_string(&addr.company, company_width)
        };
        let display_name = truncate_string(&addr.name, name_width);
        let display_addr = truncate_string(&addr.address, address_width);
        let display_chain = truncate_string(addr.chain.display_name(), chain_width);

        println!("│{:<cw$}│{:<nw$}│{:<aw$}│{:<chw$}│",
            display_company, display_name, display_addr, display_chain,
            cw = company_width, nw = name_width, aw = address_width, chw = chain_width
        );
    }

    // Print footer
    println!("├{}┴{}┴{}┴{}┤",
        "─".repeat(company_width),
        "─".repeat(name_width),
        "─".repeat(address_width),
        "─".repeat(chain_width)
    );
    let footer = format!("Total: {} address(es)", addresses.len());
    let footer_padding = table_width - 2 - footer.len();
    println!("│{}{}│", footer, " ".repeat(footer_padding));
    println!("╰{}╯\n", "─".repeat(table_width - 2));
}

pub fn render_solana_balances(company: &str, name: &str, address: &str, balances: &solana::AccountBalances, chain: &Chain) {
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
            format!("SOL Balance: {:.9} SOL (${} @ ${})", balances.sol_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("SOL Balance: {:.9} SOL (${})", balances.sol_balance, format_usd(usd_value))
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
                format!("    Mint: {}...{}", &token.mint[..20], &token.mint[token.mint.len()-20..])
            } else {
                format!("    Mint: {}", token.mint)
            };
            lines.push(mint_display);

            let balance_str = if let Some(usd_value) = token.usd_value {
                if let Some(price) = token.usd_price {
                    format!("    Balance: {:.6} (${} @ ${:.6})", token.ui_amount, format_usd(usd_value), price)
                } else {
                    format!("    Balance: {:.6} (${})", token.ui_amount, format_usd(usd_value))
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
            println!("║  {:<width$} ║", lines[line_idx], width = box_width);     // Token name
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
    println!("\n╭─────────────────────────────────────────────────────────────────────────────────╮");
    println!("│ ERROR                                                                            │");
    println!("├─────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ {}                                                                              │", error);
    println!("╰─────────────────────────────────────────────────────────────────────────────────╯\n");
}

pub fn render_success(message: &str) {
    println!("\n{}\n", message);
}

pub fn render_evm_balances(company: &str, name: &str, address: &str, balances: &evm::AccountBalances, chain: &Chain) {
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
            format!("{} Balance: {:.9} {} (${} @ ${})", native_symbol, balances.eth_balance, native_symbol, format_usd(usd_value), format_usd(price))
        } else {
            format!("{} Balance: {:.9} {} (${})", native_symbol, balances.eth_balance, native_symbol, format_usd(usd_value))
        }
    } else {
        format!("{} Balance: {:.9} {}", native_symbol, balances.eth_balance, native_symbol)
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
                    format!("    Balance: {:.6} (${} @ ${:.6})", token.ui_amount, format_usd(usd_value), price)
                } else {
                    format!("    Balance: {:.6} (${})", token.ui_amount, format_usd(usd_value))
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
            println!("║  {:<width$} ║", lines[line_idx], width = box_width);     // Token name
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

pub fn render_portfolio_summary(portfolio: &crate::PortfolioSummary) {
    const BOX_WIDTH: usize = 81;

    println!("\n╔═════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                               PORTFOLIO SUMMARY                                 ║");
    println!("╠═════════════════════════════════════════════════════════════════════════════════╣");

    // Total Portfolio Value with proper padding
    let total_value_str = format!("Total Portfolio Value: ${}", format_usd(portfolio.total_usd_value));
    let total_value_len = total_value_str.len();
    let total_padding = if total_value_len < BOX_WIDTH - 2 { BOX_WIDTH - 2 - total_value_len } else { 0 };
    println!("║  {}{:width$} ║", total_value_str, "", width = total_padding);

    if portfolio.companies.is_empty() {
        println!("╠═════════════════════════════════════════════════════════════════════════════════╣");
        println!("║  No companies found                                                             ║");
        println!("╚═════════════════════════════════════════════════════════════════════════════════╝\n");
        return;
    }

    // Sort companies by USD value (descending)
    let mut sorted_companies: Vec<_> = portfolio.companies.iter().collect();
    sorted_companies.sort_by(|a, b| b.1.total_usd_value.partial_cmp(&a.1.total_usd_value).unwrap());

    for (_, company) in sorted_companies {
        println!("╠═════════════════════════════════════════════════════════════════════════════════╣");

        // Company header
        let company_header = format!("COMPANY: {}", company.company);
        let company_header_len = company_header.len();
        let company_padding = if company_header_len < BOX_WIDTH - 2 { BOX_WIDTH - 2 - company_header_len } else { 0 };
        println!("║  {}{:width$} ║", company_header, "", width = company_padding);

        // Company total value
        let company_value_str = format!("Total Value: ${}", format_usd(company.total_usd_value));
        let company_value_len = company_value_str.len();
        let company_value_padding = if company_value_len + 2 < BOX_WIDTH - 2 { BOX_WIDTH - 2 - company_value_len - 2 } else { 0 };
        println!("║    {}{:width$} ║", company_value_str, "", width = company_value_padding);

        println!("╟─────────────────────────────────────────────────────────────────────────────────╢");

        if company.assets.is_empty() {
            println!("║      No assets found                                                            ║");
        } else {
            // Sort assets by USD value (descending)
            let mut sorted_assets: Vec<_> = company.assets.iter().collect();
            sorted_assets.sort_by(|a, b| b.1.total_usd_value.partial_cmp(&a.1.total_usd_value).unwrap());

            for (_, asset) in sorted_assets {
                // Symbol line
                let symbol_str = format!("{}:", asset.symbol);
                let symbol_len = symbol_str.len();
                let symbol_padding = if symbol_len + 4 < BOX_WIDTH - 2 { BOX_WIDTH - 2 - symbol_len - 4 } else { 0 };
                println!("║      {}{:width$} ║", symbol_str, "", width = symbol_padding);

                // Amount and USD Value on same line if USD value exists
                if asset.total_usd_value > 0.0 {
                    let detail_str = format!("{:.6} (${:})", asset.total_amount, format_usd(asset.total_usd_value));
                    let detail_len = detail_str.len();
                    let detail_padding = if detail_len + 8 < BOX_WIDTH - 2 { BOX_WIDTH - 2 - detail_len - 8 } else { 0 };
                    println!("║          {}{:width$} ║", detail_str, "", width = detail_padding);
                } else {
                    let amount_str = format!("{:.6}", asset.total_amount);
                    let amount_len = amount_str.len();
                    let amount_padding = if amount_len + 8 < BOX_WIDTH - 2 { BOX_WIDTH - 2 - amount_len - 8 } else { 0 };
                    println!("║          {}{:width$} ║", amount_str, "", width = amount_padding);
                }
            }
        }
    }

    println!("╚═════════════════════════════════════════════════════════════════════════════════╝\n");
}

pub fn render_near_balances(company: &str, name: &str, address: &str, balances: &near::AccountBalances, chain: &Chain) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    let near_line = if let Some(usd_value) = balances.near_usd_value {
        if let Some(price) = balances.near_usd_price {
            format!("NEAR Balance: {:.9} NEAR (${} @ ${})", balances.near_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("NEAR Balance: {:.9} NEAR (${})", balances.near_balance, format_usd(usd_value))
        }
    } else {
        format!("NEAR Balance: {:.9} NEAR", balances.near_balance)
    };
    lines.push(near_line);

    if let Some(total) = balances.total_usd_value {
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

    if balances.total_usd_value.is_some() {
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[5], width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_aptos_balances(company: &str, name: &str, address: &str, balances: &aptos::AccountBalances, chain: &Chain) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    let apt_line = if let Some(usd_value) = balances.apt_usd_value {
        if let Some(price) = balances.apt_usd_price {
            format!("APT Balance: {:.9} APT (${} @ ${})", balances.apt_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("APT Balance: {:.9} APT (${})", balances.apt_balance, format_usd(usd_value))
        }
    } else {
        format!("APT Balance: {:.9} APT", balances.apt_balance)
    };
    lines.push(apt_line);

    if let Some(total) = balances.total_usd_value {
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

    if balances.total_usd_value.is_some() {
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[5], width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_sui_balances(company: &str, name: &str, address: &str, balances: &sui::AccountBalances, chain: &Chain) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    let sui_line = if let Some(usd_value) = balances.sui_usd_value {
        if let Some(price) = balances.sui_usd_price {
            format!("SUI Balance: {:.9} SUI (${} @ ${})", balances.sui_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("SUI Balance: {:.9} SUI (${})", balances.sui_balance, format_usd(usd_value))
        }
    } else {
        format!("SUI Balance: {:.9} SUI", balances.sui_balance)
    };
    lines.push(sui_line);

    if let Some(total) = balances.total_usd_value {
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

    if balances.total_usd_value.is_some() {
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[5], width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_starknet_balances(company: &str, name: &str, address: &str, balances: &starknet::AccountBalances, chain: &Chain) {
    const MIN_WIDTH: usize = 79;
    let mut lines = Vec::new();

    let display_company = if company.is_empty() { "-" } else { company };
    lines.push(format!("Company: {}", display_company));
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    let eth_line = if let Some(usd_value) = balances.eth_usd_value {
        if let Some(price) = balances.eth_usd_price {
            format!("ETH Balance: {:.9} ETH (${} @ ${})", balances.eth_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("ETH Balance: {:.9} ETH (${})", balances.eth_balance, format_usd(usd_value))
        }
    } else {
        format!("ETH Balance: {:.9} ETH", balances.eth_balance)
    };
    lines.push(eth_line);

    if let Some(total) = balances.total_usd_value {
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

    if balances.total_usd_value.is_some() {
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[5], width = box_width);
    }

    println!("╚{}╝\n", "═".repeat(box_width + 2));
}
