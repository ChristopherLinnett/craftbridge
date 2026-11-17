package io.github.christopherlinnett.craftbridge.detekt

import dev.detekt.api.Config
import dev.detekt.api.ConfigValidator
import dev.detekt.api.Notification
import dev.detekt.api.RuleSet
import dev.detekt.api.RuleSetId
import dev.detekt.api.RuleSetProvider

/**
 * The build's own detekt rule set: the Kotlin rules detekt does not ship, written with detekt's own
 * rule API so the gate's Kotlin checks are all one tool reading one syntax tree (plan D-28). The
 * analyser loads it as a plugin jar through the ServiceLoader registration under `META-INF/services`,
 * from the rule classpath the convention plugin builds in build-logic; the profiles under `profiles/`
 * name which of these rules run, so a rule added here turns on nothing until a profile names it.
 */
class CraftbridgeRuleSetProvider : RuleSetProvider {
    override val ruleSetId: RuleSetId = RuleSetId(RULE_SET)

    override fun instance() =
        RuleSet(
            ruleSetId,
            listOf(
                ::NotNullAssertion,
                ::ExternalOutsidePlatformFolder,
                ::ModulePackageMismatch,
                ::FileWideSuppression,
                ::GateRuleSuppression,
                ::ErrorSuppression,
            ),
        )

    companion object {
        const val RULE_SET = "craftbridge"
    }
}

/**
 * detekt validates a run's config against the rule sets it ships, and leaves every plugin's rule set
 * out of that check: a misspelt rule under `craftbridge:` in a profile, or a misspelt `active`, would
 * pass and leave the rule off, which for a gate rule is the check gone quiet. So each section there must
 * name one of this rule set's own rules, and each says whether its rule runs. Nor does detekt question
 * a switch above the rules: `active: false` on a rule set, detekt's `style` included, turns every rule
 * in it off whatever each rule says, and `processors` switched off, or set to leave out
 * [FileWideSuppressionBackstop], turns off the one check that sees a file which silences every rule.
 * So no section of the run's config says anything but `active: true`, and the backstop runs. Nor may a
 * section hold one of the filters detekt keeps a running rule from code by
 * ([FILTERS_OVER_WHAT_RULES_READ]), for the same reason. detekt loads this validator from the same jar
 * as the rules, through the ServiceLoader registration beside theirs.
 */
class CraftbridgeConfigValidator : ConfigValidator {
    override val id: String = "CraftbridgeConfigValidator"

    override fun validate(config: Config): Collection<Notification> =
        (
            switchesAboveTheRules(config) +
                filtersOverWhatRulesRead(config) +
                ruleSectionProblems(config.subConfig(CraftbridgeRuleSetProvider.RULE_SET))
        ).map { problem -> Notification(problem, Notification.Level.Error) }

    private fun switchesAboveTheRules(config: Config): List<String> {
        val backstop = FileWideSuppressionBackstop().id
        return config.sections().flatMap { (name, section) ->
            val active = section.valueOrNull<Any>(Config.ACTIVE_KEY)
            listOfNotNull(
                "$name>active is `$active`, which switches off everything under $name whatever each part says"
                    .takeIf { active != null && active != true },
                "processors>exclude leaves out $backstop, the one check that sees a file which silences every rule"
                    .takeIf { name == "processors" && backstop in section.valueOrDefault("exclude", emptyList<String>()) },
            )
        }
    }

    /**
     * The keys detekt keeps a running rule from code by: `excludes` and `includes` choose the files a rule
     * reads at all, read on the rule and on its rule set alike, and `ignoreAnnotated` and `ignoreFunction`
     * drop what the rule finds under an annotation or inside a named function, read on the rule alone.
     * detekt's own validation lets every one of them stand on a rule, `excludes` and `includes` on any
     * rule set too, and checks a plugin's rule set not at all, so a run whose rule reads no file, or drops
     * what it found, passes with that rule still saying `active: true`. So no section of a run's config
     * holds one, at either level: on a rule set, `ignoreAnnotated` or `ignoreFunction` does nothing and is
     * refused all the same, since it reads as a filter there too. Which rules run, and over which code, is
     * what a run's profiles say — the base, the written and the shipped ones, and for the bindings the base
     * alone — and a rule skipping a file, an annotation or a function is no part of that. [CONFIG_SECTION]
     * is detekt's own switches and no rule set: its `excludes` names the paths validation leaves out,
     * which the config the build writes last pins empty.
     */
    private fun filtersOverWhatRulesRead(config: Config): List<String> =
        config
            .sections()
            .filterNot { (name, _) -> name == CONFIG_SECTION }
            .flatMap { (name, section) ->
                filterProblems(name, section) +
                    section.sections().flatMap { (rule, ruleConfig) -> filterProblems("$name>$rule", ruleConfig) }
            }

    private fun filterProblems(
        path: String,
        section: Config,
    ): List<String> =
        FILTERS_OVER_WHAT_RULES_READ
            .filter { section.valueOrNull<Any>(it) != null }
            .map { filter ->
                "$path>$filter is one of the filters detekt keeps a running rule from code by, which no run's " +
                    "config holds: which rules run, and over which code, is what a run's profiles name"
            }

    private fun ruleSectionProblems(ruleSet: Config): List<String> {
        val rules =
            CraftbridgeRuleSetProvider()
                .instance()
                .rules.keys
                .map { it.value }
                .toSet()
        // Everything beside a rule's section is a way to leave a rule off unseen: a rule written with no
        // body is no section at all. The set's own `active` is judged with every other section's, and a
        // filter on the set with every other filter, so each is refused once, for what it is.
        return ruleSet
            .subConfigKeys()
            .filter { it != Config.ACTIVE_KEY && it !in FILTERS_OVER_WHAT_RULES_READ }
            .mapNotNull { key ->
                when {
                    ruleSet.valueOrNull<Any>(key) !is Map<*, *> -> {
                        "is no section with `active: true` or `active: false`, so the rule it names is left off unseen"
                    }

                    key.substringBefore('/') !in rules -> {
                        "is no rule of this rule set, whose rules are ${rules.sorted().joinToString()}"
                    }

                    ruleSet.subConfig(key).valueOrNull<Any>(Config.ACTIVE_KEY) !is Boolean -> {
                        "says no `active: true` or `active: false`, so whether the rule runs is left to chance"
                    }

                    else -> {
                        null
                    }
                }?.let { problem -> "craftbridge>$key $problem" }
            }
    }

    private companion object {
        /** detekt's own switches, the one section of a run's config that is no rule set. */
        const val CONFIG_SECTION = "config"

        /** The keys [filtersOverWhatRulesRead] refuses; detekt has no constant for the last of them. */
        val FILTERS_OVER_WHAT_RULES_READ =
            listOf(Config.EXCLUDES_KEY, Config.INCLUDES_KEY, Config.IGNORE_ANNOTATED_KEY, "ignoreFunction")
    }
}
