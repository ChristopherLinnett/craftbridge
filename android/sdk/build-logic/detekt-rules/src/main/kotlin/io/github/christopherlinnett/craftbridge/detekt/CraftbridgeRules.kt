package io.github.christopherlinnett.craftbridge.detekt

// The gate's Kotlin checks detekt does not ship, read from Kotlin's own syntax tree (plan D-28).
// The rules share their header — the same detekt api they build a rule from — so they are classes in
// this one file: split over a file each, that header would be the same lines over and over, which is
// the duplication AGENTS.md §5 refuses. The profiles in detekt-rules/profiles name which run where.

import dev.detekt.api.Config
import dev.detekt.api.Detektion
import dev.detekt.api.Entity
import dev.detekt.api.FileProcessListener
import dev.detekt.api.Finding
import dev.detekt.api.Issue
import dev.detekt.api.Location
import dev.detekt.api.RequiresAnalysisApi
import dev.detekt.api.Rule
import dev.detekt.api.RuleSetId
import dev.detekt.api.SetupContext
import dev.detekt.api.config
import dev.detekt.core.suppressors.isSuppressedBy
import org.jetbrains.kotlin.analysis.api.analyze
import org.jetbrains.kotlin.analysis.api.types.symbol
import org.jetbrains.kotlin.lexer.KtTokens
import org.jetbrains.kotlin.name.ClassId
import org.jetbrains.kotlin.psi.KtAnnotated
import org.jetbrains.kotlin.psi.KtAnnotationEntry
import org.jetbrains.kotlin.psi.KtDeclaration
import org.jetbrains.kotlin.psi.KtFile
import org.jetbrains.kotlin.psi.KtPostfixExpression
import org.jetbrains.kotlin.psi.psiUtil.getStrictParentOfType
import java.nio.file.Path
import java.util.concurrent.ConcurrentLinkedQueue

/**
 * `!!` throws a NullPointerException where library code fails with a documented exception instead
 * (AGENTS.md §3). The ban is on the operator itself, read from the syntax tree, so it holds whatever
 * the operand's type: detekt's UnsafeCallOnNullableType reports `!!` only on a type marked nullable,
 * which leaves out a Java value of platform type (`System.getProperty("k")!!`) — the very place a
 * null arrives unannounced — and any operand whose type the analysis could not settle.
 */
class NotNullAssertion(
    config: Config,
) : Rule(config, "`!!` throws a NullPointerException: library code fails with a documented exception instead") {
    override fun visitPostfixExpression(expression: KtPostfixExpression) {
        super.visitPostfixExpression(expression)
        if (expression.operationToken == KtTokens.EXCLEXCL) {
            report(
                Finding(
                    Entity.from(expression.operationReference),
                    "`!!` throws a NullPointerException: fail with a documented exception instead (AGENTS.md §3)",
                ),
            )
        }
    }
}

/**
 * A Kotlin `external` declaration is a JNI binding: every call through it crosses into native code no
 * compiler can see, so the declarations live only under a `platform/` folder — in every source set,
 * the test sets included (AGENTS.md §4). Tests take the never-crash shortcuts, never this: the native
 * surface is the one thing a reviewer has to be able to find in one place, and a test is exactly where
 * an unreviewed binding gets written first.
 */
class ExternalOutsidePlatformFolder(
    config: Config,
) : Rule(config, "external (JNI) declarations live only under a platform/ folder, in every source set") {
    // Read when a file is judged, as the package rule reads its packages: the convention plugin writes
    // the module's folder into the config each run is given, and a run without it is wired wrong.
    private val moduleDirectory: String by config("")

    // Every kind of declaration the modifier can sit on — a function, a property, an accessor alone
    // (`val version: Int external get`) — reaches the tree's one visit for declarations.
    override fun visitDeclaration(declaration: KtDeclaration) {
        super.visitDeclaration(declaration)
        if (!declaration.hasModifier(KtTokens.EXTERNAL_KEYWORD)) {
            return
        }
        val module =
            moduleDirectory.ifEmpty {
                error("ExternalOutsidePlatformFolder has no moduleDirectory: craftbridge.android.library writes the module's folder")
            }
        if (!underPlatformFolder(module, declaration.containingKtFile.virtualFilePath)) {
            report(
                Finding(
                    Entity.from(declaration),
                    "`external` (JNI) here, outside a `platform/` folder: native code the compiler cannot see " +
                        "stays in one place a reviewer can find (AGENTS.md §4)",
                ),
            )
        }
    }
}

/**
 * Does `virtualFilePath` sit under a `platform` folder of its source set? A module's sources sit at
 * `src/<source set>/<kotlin or java>/` below the module's folder — AGP's layout, which no module build
 * file can change — so the path is read from below `moduleDirectory`: the source root is the third
 * folder there, and `platform` counts only between it and the file. Nothing above the module counts, a
 * `platform`, `src`, `kotlin` or `java` folder somewhere above the checkout included, and a package below
 * the root named `kotlin` or `java` is only a package. A file outside that shape is outside `platform/`.
 */
internal fun underPlatformFolder(
    moduleDirectory: String,
    virtualFilePath: String,
): Boolean {
    val parts =
        virtualFilePath.removePrefix("${moduleDirectory.trimEnd('/')}/").takeIf { it != virtualFilePath }?.split('/') ?: return false
    val inSourceRoot = parts.size > 3 && parts[0] == "src" && (parts[2] == "kotlin" || parts[2] == "java")
    return inSourceRoot && parts.subList(3, parts.lastIndex).contains("platform")
}

/**
 * Every file of a module states the module's package — `io.github.christopherlinnett.craftbridge.<module>`
 * for the code written by hand, `io.github.christopherlinnett.craftbridge.ffi.<module>` for the generated
 * bindings (AGENTS.md §9) — so an artifact never ships a class that points at a package some other
 * module, or nothing, wrote.
 *
 * The expected packages are not this rule's to know: the convention plugin writes the module's own
 * namespace, or its bindings' spelling of it, into the config each run is given, so the rule repeats
 * nothing and a module cannot change what it is checked against from a file it is being checked on.
 * A file that declares no package is the same violation in a louder form: there is nothing to check.
 */
class ModulePackageMismatch(
    config: Config,
) : Rule(config, "every file declares its module's package") {
    // Read when a file is judged, never when the rule is built: detekt builds every rule of a rule set
    // with an empty config to learn its name, and a run that reaches a file with no list is wired to a
    // broken config, so it fails loudly rather than passing every file.
    private val expectedPackages: List<String> by config(emptyList())

    override fun visitKtFile(file: KtFile) {
        // No call to super on purpose: the package belongs to the file itself; its contents say nothing.
        val expected =
            expectedPackages.ifEmpty {
                error("ModulePackageMismatch has no expectedPackages: craftbridge.android.library writes them from the module's namespace")
            }
        // A file with no package directive reads one with an empty name, and an empty name is the finding.
        val declared = file.packageDirective?.qualifiedName
        val message =
            when {
                declared.isNullOrEmpty() -> {
                    "the file declares no package: every Kotlin file states its module's package (AGENTS.md §9)"
                }

                expected.none { declared == it || declared.startsWith("$it.") } -> {
                    "`$declared` is not this module's package: expected ${expected.joinToString(" or ")} (AGENTS.md §9)"
                }

                else -> {
                    return
                }
            }
        report(Finding(Entity.atPackageOrFirstDecl(file), message))
    }
}

/** The annotations Kotlin reads as a suppression: its own `Suppress` and Java's `SuppressWarnings`, however qualified. */
private val suppressionAnnotations = setOf("Suppress", "SuppressWarnings", "kotlin.Suppress", "java.lang.SuppressWarnings")

private fun KtAnnotationEntry.isSuppression() = typeReference?.text?.replace("`", "") in suppressionAnnotations

/** The file's suppressions detekt itself reads by name, the only ones that can keep a rule off a file. */
private fun KtFile.fileWideSuppressions() = annotationEntries.filter { it.isSuppression() }

private fun fileWideMessage(annotation: KtAnnotationEntry) =
    "`${annotation.text}` switches checks off on the whole file: switch the one item off, with a comment saying why (AGENTS.md §10)"

/** The classes the compiler reads as a suppression, whatever name a file gives them. */
private val suppressionClasses = setOf(ClassId.fromString("kotlin/Suppress"), ClassId.fromString("java/lang/SuppressWarnings"))

private fun KtAnnotationEntry.resolvesToSuppression() = analyze(this) { typeReference?.type?.symbol?.classId } in suppressionClasses

/**
 * A suppression written over a whole file switches checks off on every line of it — the compiler's
 * warnings, ktlint's rules, Lint's checks and detekt's — which is what AGENTS.md §10 refuses: a check
 * comes off on the one item that needs it, with a comment saying why, or not at all. `@file:Suppress`
 * is that switch whatever it names, so its presence is itself the finding, however it is spelled: as
 * `Suppress`, as the Java spelling `SuppressWarnings` Kotlin reads too, fully qualified, or under an
 * import alias or a typealias, which the compiler honours as the class it resolves to — so the class
 * an annotation resolves to is asked as well as the name it is written with.
 *
 * detekt runs no rule at all over a file whose own `@file:Suppress` names that rule, its rule set or
 * `all` — the very switch this rule exists to catch — so [FileWideSuppressionBackstop] reports the
 * files this rule is not let read.
 */
class FileWideSuppression(
    config: Config,
) : Rule(config, "a suppression written over a whole file switches every check off on every line of it"),
    RequiresAnalysisApi {
    override fun visitKtFile(file: KtFile) {
        // No call to super on purpose: the file's header is all this rule reads; its contents are the
        // business of the other rules, each of which the engine visits with its own visitor.
        for (annotation in file.annotationEntries.filter { it.isSuppression() || it.resolvesToSuppression() }) {
            report(Finding(Entity.from(annotation), fileWideMessage(annotation)))
        }
    }
}

/**
 * [FileWideSuppression] for the files detekt does not let it read: those whose own `@file:Suppress`
 * silences it, which detekt decides before any rule runs, by the same check asked here. A processor
 * sees every file, suppressed or not, so it files the rule's findings for those files under the rule's
 * own name, where the run counts them like any other; on every other file the rule reports for itself,
 * so nothing is reported twice. In a run that does not switch the rule on, it adds nothing.
 */
class FileWideSuppressionBackstop : FileProcessListener {
    override val id: String = "FileWideSuppressionBackstop"

    private val ruleId = FileWideSuppression::class.java.simpleName
    private val ruleSet = RuleSetId(CraftbridgeRuleSetProvider.RULE_SET)
    private lateinit var basePath: Path
    private lateinit var aliases: Set<String>
    private val unread = ConcurrentLinkedQueue<Pair<KtAnnotationEntry, Location>>()

    override fun init(context: SetupContext) {
        basePath = context.basePath
        aliases =
            context.config
                .subConfig(ruleSet.value)
                .subConfig(ruleId)
                .valueOrDefault(Config.ALIASES_KEY, emptyList<String>())
                .toSet()
    }

    override fun onProcess(file: KtFile) {
        if (file.isSuppressedBy(ruleId, aliases, ruleSet)) {
            file.fileWideSuppressions().forEach { unread += it to Location.from(it) }
        }
    }

    override fun onFinish(
        files: List<KtFile>,
        result: Detektion,
    ): Detektion {
        val rule = result.rules.firstOrNull { it.id == ruleId && it.ruleSetId == ruleSet && it.active } ?: return result
        val issues =
            unread.map { (annotation, at) ->
                Issue(
                    rule,
                    Issue.Entity(
                        Entity.from(annotation).signature,
                        Issue.Location(at.source, at.endSource, at.text, basePath.relativize(at.path)),
                    ),
                    emptyList(),
                    fileWideMessage(annotation),
                    rule.severity,
                    emptyList(),
                )
            }
        return Detektion(result.issues + issues, result.rules, result.notifications, result.metrics, result.userData)
    }
}

/**
 * A suppression on one declaration that silences a rule this run enforces. Every rule detekt runs in
 * the gate is a gate rule — the never-crash bans of AGENTS.md §3, which §10 never lets an item switch
 * off, and the layout rules of §4 and §9, which have no false positive to excuse — so a suppression
 * detekt would honour for any of them is refused, while one naming only the compiler's, ktlint's or
 * Lint's checks, which §10 allows on one item with its reason, passes.
 *
 * Which annotation silences which rule is detekt's own question, so it is asked of detekt's own
 * suppression check with each active rule's id, rule set and aliases — read from this run's config,
 * so no list of rules is written twice. A finding is filed against the file, not the annotated
 * declaration: filed there, the very suppression it reports would hide it.
 */
class GateRuleSuppression(
    config: Config,
) : Rule(config, "a suppression on a declaration switches off a rule the gate enforces") {
    // The run's config holds rule sets of rules, beside settings that are plain values; only a section
    // is descended into.
    private val activeRules: List<ActiveRule> by lazy {
        val root = config.parent?.parent ?: return@lazy emptyList()
        root.sections().flatMap { (ruleSet, ruleSetConfig) ->
            ruleSetConfig
                .sections()
                .filter { (_, ruleConfig) -> ruleConfig.valueOrDefault(Config.ACTIVE_KEY, false) }
                .map { (rule, ruleConfig) ->
                    ActiveRule(rule, RuleSetId(ruleSet), ruleConfig.valueOrDefault(Config.ALIASES_KEY, emptyList<String>()).toSet())
                }
        }
    }

    override fun visitAnnotationEntry(annotationEntry: KtAnnotationEntry) {
        super.visitAnnotationEntry(annotationEntry)
        val owner = annotationEntry.getStrictParentOfType<KtAnnotated>() ?: return
        // A file's own suppression is FileWideSuppression's finding, whatever it names. Of an item's
        // annotations only its first suppression is read, so an item reports once: detekt reads all of
        // them together, and the message names every rule they silence.
        if (owner is KtFile || !annotationEntry.isSuppression() ||
            owner.annotationEntries.firstOrNull { it.isSuppression() } != annotationEntry
        ) {
            return
        }
        val enclosing = owner.getStrictParentOfType<KtAnnotated>()
        // A rule an enclosing item already silences is that item's finding, reported there.
        val silenced =
            activeRules.filter { rule ->
                owner.isSuppressedBy(rule.id, rule.aliases, rule.ruleSet) &&
                    enclosing?.isSuppressedBy(rule.id, rule.aliases, rule.ruleSet) != true
            }
        if (silenced.isNotEmpty()) {
            val file = annotationEntry.containingKtFile
            report(
                Finding(
                    Entity(Entity.from(annotationEntry).signature, Location.from(annotationEntry), file),
                    "this suppression switches off ${silenced.joinToString { it.id }}, which the gate enforces on every item: " +
                        "fix the code instead (AGENTS.md §3, §10)",
                ),
            )
        }
    }

    private class ActiveRule(
        val id: String,
        val ruleSet: RuleSetId,
        val aliases: Set<String>,
    )
}
